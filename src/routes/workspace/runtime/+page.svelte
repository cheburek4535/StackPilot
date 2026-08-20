<script lang="ts">
  import { onDestroy } from "svelte";
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import Modal from "$lib/components/ui/Modal.svelte";
  import { workspaceContext, reloadWorkspaceContext } from "$lib/modules/workspace/context";
  import {
    listProcesses,
    refreshProcess,
    killProcess,
    getProcessLogs,
  } from "$lib/modules/workspace/api";
  import type { TrackedProcess, ProcessLogs } from "$lib/modules/workspace/types";
  import {
    statusTone,
    statusLabel,
    statusIcon,
    formatDuration,
    formatStarted,
  } from "$lib/modules/workspace/status";
  import { notifySuccess, notifyError } from "$lib/core/toasts";

  let processes = $state<TrackedProcess[]>([]);
  let dataLoaded = $state(false);
  let refreshing = $state(false);
  let errorMsg = $state("");

  let logProcId = $state<string | null>(null);
  let logProcName = $state("");
  let logs = $state<ProcessLogs | null>(null);
  let logsLoading = $state(false);
  let logsError = $state("");

  let autoRefreshId: ReturnType<typeof setInterval> | null = null;

  const project = $derived($workspaceContext.project);
  const wsLoading = $derived($workspaceContext.loading);
  const wsError = $derived($workspaceContext.error);

  $effect(() => {
    if (project && !dataLoaded) {
      dataLoaded = true;
      loadProcesses();
      startAutoRefresh();
    } else if (!project) {
      dataLoaded = false;
      processes = [];
      stopAutoRefresh();
    }
  });

  onDestroy(() => {
    stopAutoRefresh();
  });

  function startAutoRefresh() {
    if (autoRefreshId) return;
    autoRefreshId = setInterval(() => {
      refreshAllStatuses();
    }, 3000);
  }

  function stopAutoRefresh() {
    if (autoRefreshId) {
      clearInterval(autoRefreshId);
      autoRefreshId = null;
    }
  }

  async function loadProcesses() {
    try {
      processes = await listProcesses();
    } catch (e) {
      errorMsg = `Failed to load processes: ${e}`;
    }
  }

  async function refreshAllStatuses() {
    if (refreshing) return;
    refreshing = true;
    for (const proc of processes) {
      try {
        await refreshProcess(proc.id);
      } catch {
        // process may be gone — the list reload below reconciles it
      }
    }
    await loadProcesses();
    refreshing = false;
  }

  async function handleRefresh(id: string) {
    try {
      await refreshProcess(id);
      await loadProcesses();
    } catch (e) {
      errorMsg = `Refresh failed: ${e}`;
    }
  }

  async function handleKill(id: string) {
    try {
      await killProcess(id);
      notifySuccess("Process killed", `PID ${processes.find((p) => p.id === id)?.pid ?? ""}`);
      await loadProcesses();
    } catch (e) {
      notifyError("Kill process", `Failed to kill: ${e}`);
    }
  }

  async function openLogs(id: string) {
    const proc = processes.find((p) => p.id === id);
    logProcId = id;
    logProcName = proc?.label ?? id;
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

  function closeLogs() {
    logProcId = null;
    logs = null;
    logsError = "";
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title="Runtime"
    description="Processes under this workspace — status, logs and control. Data comes from listProcesses()/refreshProcess()/killProcess()/getProcessLogs()."
    icon="play"
  >
    {#snippet actions()}
      <Button
        variant="secondary"
        size="sm"
        icon="refresh"
        loading={refreshing}
        onclick={refreshAllStatuses}
      >
        Refresh
      </Button>
    {/snippet}
  </PageHeader>

  {#if wsLoading}
    <LoadingState label="Loading workspace…" />
  {:else if wsError}
    <ErrorState
      title="Failed to load workspace"
      message={wsError}
      retry={() => {
        reloadWorkspaceContext();
        dataLoaded = false;
        loadProcesses();
      }}
    />
  {:else if !project}
    <EmptyState
      icon="folder"
      title="No project is open"
      description="Open a project to see its runtime processes here."
    />
  {:else}
    {#if errorMsg}
      <div class="sp-banner sp-banner-err">{errorMsg}</div>
    {/if}

    {#if !dataLoaded}
      <LoadingState label="Loading processes…" />
    {:else if processes.length === 0}
      <EmptyState
        icon="terminal"
        title="No processes"
        description="Nothing has been spawned for this workspace yet. Use the DevLauncher Process Manager to spawn or run a profile."
      >
        {#snippet action()}
          <Button variant="secondary" icon="terminal" href="/devlauncher/processes">
            Open Process Manager
          </Button>
        {/snippet}
      </EmptyState>
    {:else}
      <div class="sp-proc-list">
        {#each processes as p (p.id)}
          <Card padding="md">
            <div class="sp-proc-card">
              <div class="sp-proc-main">
                <div class="sp-proc-head">
                  <span class="sp-proc-icon sp-proc-icon-{statusIcon(p.status)}" aria-hidden="true">
                    <Icon name={statusIcon(p.status)} size={15} />
                  </span>
                  <strong class="sp-proc-label">{p.label}</strong>
                  <Badge tone={statusTone(p.status)} dot={p.status === "Running"}>
                    {statusLabel(p.status)}
                  </Badge>
                </div>
                <div class="sp-proc-meta">
                  <span>PID <code>{p.pid}</code></span>
                  <span class="sp-sep">·</span>
                  <span>{formatDuration(p.duration_secs)}</span>
                  <span class="sp-sep">·</span>
                  <span>started {formatStarted(p.started_at)}</span>
                  {#if p.restarts > 0}
                    <span class="sp-sep">·</span>
                    <span class="sp-restarts">restarts {p.restarts}</span>
                  {/if}
                </div>
                {#if p.last_error}
                  <div class="sp-proc-error">{p.last_error}</div>
                {/if}
              </div>
              <div class="sp-proc-actions">
                <Button
                  size="sm"
                  variant="ghost"
                  icon="terminal"
                  onclick={() => openLogs(p.id)}
                >
                  Logs
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  icon="refresh"
                  label="Refresh status"
                  onclick={() => handleRefresh(p.id)}
                />
                <Button
                  size="sm"
                  variant="danger"
                  icon="x"
                  disabled={p.status !== "Running"}
                  onclick={() => handleKill(p.id)}
                >
                  Kill
                </Button>
              </div>
            </div>
          </Card>
        {/each}
      </div>
    {/if}
  {/if}
</PageContainer>

<Modal
  open={logProcId !== null}
  onclose={closeLogs}
  title="Process logs"
  description={logProcName}
  size="lg"
>
  {#if logsLoading}
    <div class="sp-log-hint">Loading logs…</div>
  {:else if logsError}
    <div class="sp-log-hint sp-log-hint-err">{logsError}</div>
  {:else if logs}
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
</Modal>

<style>
  .sp-banner {
    padding: var(--sp-3) var(--sp-4);
    border-radius: var(--sp-radius-md);
    font-size: var(--sp-fs-sm);
    margin-bottom: var(--sp-4);
  }

  .sp-banner-err {
    color: var(--sp-danger);
    background: rgba(248, 113, 113, 0.12);
    border: 1px solid rgba(248, 113, 113, 0.35);
  }

  .sp-proc-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .sp-proc-card {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--sp-4);
  }

  .sp-proc-main {
    flex: 1;
    min-width: 0;
  }

  .sp-proc-head {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-bottom: var(--sp-1);
  }

  .sp-proc-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: var(--sp-radius-sm);
    flex-shrink: 0;
  }

  .sp-proc-icon-play {
    color: var(--sp-success);
    background: rgba(163, 230, 53, 0.12);
  }

  .sp-proc-icon-check {
    color: var(--sp-blue);
    background: rgba(96, 165, 250, 0.12);
  }

  .sp-proc-icon-x,
  .sp-proc-icon-alert {
    color: var(--sp-danger);
    background: rgba(248, 113, 113, 0.12);
  }

  .sp-proc-label {
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sp-proc-meta {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-wrap: wrap;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
  }

  .sp-proc-meta code {
    font-size: var(--sp-fs-xs);
    background: var(--sp-code-bg);
    padding: 0.05rem 0.35rem;
    border-radius: var(--sp-radius-xs);
    color: var(--sp-text-1);
  }

  .sp-sep {
    color: var(--sp-text-3);
  }

  .sp-restarts {
    background: rgba(251, 191, 36, 0.14);
    color: var(--sp-warning);
    padding: 0.05rem 0.4rem;
    border-radius: var(--sp-radius-xs);
  }

  .sp-proc-error {
    margin-top: var(--sp-2);
    font-size: var(--sp-fs-xs);
    color: var(--sp-danger);
    background: rgba(248, 113, 113, 0.12);
    border: 1px solid rgba(248, 113, 113, 0.25);
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--sp-radius-sm);
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 4rem;
    overflow-y: auto;
  }

  .sp-proc-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-shrink: 0;
  }

  /* log viewer */

  .sp-log-view {
    background: var(--sp-bg-0);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    padding: var(--sp-3);
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    line-height: var(--sp-lh-normal);
    max-height: 24rem;
    overflow-y: auto;
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
    padding: var(--sp-8) var(--sp-4);
    text-align: center;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
    font-style: italic;
  }

  .sp-log-hint-err {
    color: var(--sp-danger);
  }
</style>
