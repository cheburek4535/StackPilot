<script lang="ts">
  import { goto } from "$app/navigation";
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import {
    workspaceContext,
    reloadWorkspaceContext,
  } from "$lib/modules/workspace/context";
  import { listProcesses, getSessionInfo } from "$lib/modules/workspace/api";
  import type { TrackedProcess, SessionInfo } from "$lib/modules/workspace/types";
  import {
    statusTone,
    statusLabel,
    statusIcon,
    formatDuration,
    formatDateTime,
  } from "$lib/modules/workspace/status";
  import { openProject } from "$lib/core/integration";
  import { recentProjects } from "$lib/core/recent";
  import type { RecentProjectRef } from "$lib/core/recent";
  import { notifySuccess, notifyError } from "$lib/core/toasts";

  let processes = $state<TrackedProcess[]>([]);
  let session = $state<SessionInfo | null>(null);
  let dataLoaded = $state(false);
  let openingPath = $state<string | null>(null);

  const project = $derived($workspaceContext.project);
  const wsLoading = $derived($workspaceContext.loading);
  const wsError = $derived($workspaceContext.error);

  $effect(() => {
    if (project && !dataLoaded) {
      dataLoaded = true;
      loadData();
    } else if (!project) {
      dataLoaded = false;
      processes = [];
      session = null;
    }
  });

  async function loadData() {
    const [procs, sess] = await Promise.all([
      listProcesses().catch(() => [] as TrackedProcess[]),
      getSessionInfo().catch(() => null),
    ]);
    processes = procs;
    session = sess;
  }

  const runningProcs = $derived(processes.filter((p) => p.status === "Running"));
  const runningCount = $derived(runningProcs.length);
  const erroredCount = $derived(
    processes.filter((p) => {
      if (p.status === "Crashed") return true;
      if (
        typeof p.status === "object" &&
        "Exited" in p.status &&
        p.status.Exited !== 0
      )
        return true;
      return false;
    }).length,
  );
  const restarts = $derived(processes.reduce((sum, p) => sum + p.restarts, 0));

  async function openRecent(ref: RecentProjectRef) {
    openingPath = ref.path;
    try {
      await openProject(ref.path);
      await reloadWorkspaceContext();
      notifySuccess("Project opened", ref.name);
    } catch (e) {
      notifyError("Open project", `Failed to open ${ref.path}: ${e}`);
    }
    openingPath = null;
  }

  function formatWhen(iso: string): string {
    if (!iso) return "";
    const ms = Date.now() - Date.parse(iso);
    if (!Number.isFinite(ms)) return "";
    const mins = Math.floor(ms / 60000);
    if (mins < 1) return "just now";
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    return `${Math.floor(hours / 24)}d ago`;
  }

  function reloadAll() {
    dataLoaded = false;
    processes = [];
    session = null;
    reloadWorkspaceContext();
    loadData();
  }
</script>

<PageContainer width="wide">
  {#if wsLoading}
    <LoadingState label="Loading workspace…" />
  {:else if wsError}
    <ErrorState
      title="Failed to load workspace"
      message={wsError}
      retry={reloadAll}
    />
  {:else if !project}
    <div class="sp-empty-wrap">
      {#snippet emptyAction()}
        <Button variant="primary" icon="layers" href="/devlauncher">
          Open DevLauncher
        </Button>
        <Button variant="secondary" icon="sparkles" href="/create">
          Project Creator
        </Button>
      {/snippet}
      <EmptyState
        icon="folder"
        title="No project is open"
        description="Open a project to start a workspace. A project becomes the current workspace only when you explicitly open it — local history is shown below, never auto-loaded."
        action={emptyAction}
      />
    </div>

    {#if $recentProjects.length > 0}
      <div class="sp-recent-section">
        <Card
          title="Recent projects"
          description="UI-local history of projects you opened or created — these are references, not the current workspace. Open one to make it current."
        >
          <div class="sp-recent-list">
            {#each $recentProjects as ref}
              <div class="sp-recent-row">
                <div class="sp-recent-main">
                  <div class="sp-recent-name-row">
                    <strong class="sp-recent-name">{ref.name}</strong>
                    <Badge tone="neutral">{ref.source}</Badge>
                  </div>
                  <span class="sp-recent-path">{ref.path}</span>
                  <span class="sp-recent-when">{formatWhen(ref.at)}</span>
                </div>
                <Button
                  size="sm"
                  variant="primary"
                  icon="layers"
                  loading={openingPath === ref.path}
                  disabled={openingPath !== null}
                  onclick={() => openRecent(ref)}
                >
                  Open
                </Button>
              </div>
            {/each}
          </div>
        </Card>
      </div>
    {/if}
  {:else}
    <PageHeader
      title="Overview"
      description="Project context, session and the processes running under this workspace."
      icon="layers"
    />

    <Card variant="elevated" padding="lg">
      <div class="sp-hero">
        <div class="sp-hero-main">
          <div class="sp-hero-head">
            <h2 class="sp-hero-name">{project.profile_name}</h2>
            <Badge tone="violet">current</Badge>
          </div>
          {#if project.description}
            <p class="sp-hero-desc">{project.description}</p>
          {/if}
          {#if project.project_path}
            <p class="sp-hero-path">{project.project_path}</p>
          {/if}
          {#if project.stack.length > 0}
            <div class="sp-hero-tags">
              {#each project.stack as tech}
                <Badge tone="blue">{tech}</Badge>
              {/each}
            </div>
          {/if}
          <p class="sp-hero-opened">
            Opened {formatDateTime(project.opened_at)}
          </p>
        </div>
        {#if project.project_path}
          <Button
            variant="secondary"
            icon="folder"
            onclick={() => goto("/workspace/files")}
          >
            Browse files
          </Button>
        {/if}
      </div>
    </Card>

    <div class="sp-stats">
      <div class="sp-stat">
        <span class="sp-stat-icon sp-stat-icon-lime" aria-hidden="true">
          <Icon name="play" size={16} />
        </span>
        <div class="sp-stat-text">
          <span class="sp-stat-value">{runningCount}</span>
          <span class="sp-stat-label">Running processes</span>
        </div>
      </div>
      <div class="sp-stat">
        <span class="sp-stat-icon sp-stat-icon-violet" aria-hidden="true">
          <Icon name="terminal" size={16} />
        </span>
        <div class="sp-stat-text">
          <span class="sp-stat-value">{processes.length}</span>
          <span class="sp-stat-label">Total processes</span>
        </div>
      </div>
      <div class="sp-stat">
        <span class="sp-stat-icon sp-stat-icon-amber" aria-hidden="true">
          <Icon name="refresh" size={16} />
        </span>
        <div class="sp-stat-text">
          <span class="sp-stat-value">{restarts}</span>
          <span class="sp-stat-label">Restarts</span>
        </div>
      </div>
      <div class="sp-stat">
        <span class="sp-stat-icon sp-stat-icon-cyan" aria-hidden="true">
          <Icon name="clock" size={16} />
        </span>
        <div class="sp-stat-text">
          <span class="sp-stat-value">
            {session ? formatDuration(session.duration_secs) : "—"}
          </span>
          <span class="sp-stat-label">Session uptime</span>
        </div>
      </div>
    </div>

    <div class="sp-grid">
      <Card
        title="Running processes"
        description="Live list from listProcesses(). Manage them from Runtime."
      >
        {#if runningProcs.length === 0}
          <div class="sp-inline-empty">
            <p>No processes are running.</p>
            <Button
              size="sm"
              variant="secondary"
              icon="terminal"
              onclick={() => goto("/workspace/runtime")}
            >
              Open Runtime
            </Button>
          </div>
        {:else}
          <div class="sp-proc-list">
            {#each runningProcs as p}
              <div class="sp-proc-row">
                <span class="sp-proc-icon" aria-hidden="true">
                  <Icon name={statusIcon(p.status)} size={14} />
                </span>
                <div class="sp-proc-main">
                  <span class="sp-proc-label">{p.label}</span>
                  <span class="sp-proc-meta">
                    PID {p.pid} · {formatDuration(p.duration_secs)}
                  </span>
                </div>
                <Badge tone={statusTone(p.status)}>{statusLabel(p.status)}</Badge>
              </div>
            {/each}
          </div>
          {#if erroredCount > 0}
            <p class="sp-proc-warn">
              {erroredCount} process{erroredCount === 1 ? "" : "es"} in an errored
              state — see Problems.
            </p>
          {/if}
        {/if}
      </Card>

      <Card
        title="Session"
        description="Backend getSessionInfo() — the current development session."
      >
        {#if !session}
          <div class="sp-inline-empty">
            <p>No session has been started for this project yet.</p>
          </div>
        {:else}
          <div class="sp-session-grid">
            <div class="sp-session-item">
              <span class="sp-session-label">Started</span>
              <span class="sp-session-value">
                {formatDateTime(session.started_at)}
              </span>
            </div>
            <div class="sp-session-item">
              <span class="sp-session-label">Duration</span>
              <span class="sp-session-value">
                {formatDuration(session.duration_secs)}
              </span>
            </div>
            <div class="sp-session-item">
              <span class="sp-session-label">Processes</span>
              <span class="sp-session-value">{session.process_count}</span>
            </div>
            <div class="sp-session-item">
              <span class="sp-session-label">Errors</span>
              <span class="sp-session-value" class:sp-session-err={session.error_count > 0}>
                {session.error_count}
              </span>
            </div>
          </div>
        {/if}
      </Card>
    </div>
  {/if}
</PageContainer>

<style>
  .sp-empty-wrap {
    margin: var(--sp-4) 0;
  }

  .sp-recent-section {
    margin-top: var(--sp-4);
  }

  .sp-recent-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-recent-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .sp-recent-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-recent-name-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .sp-recent-name {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-recent-path {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sp-recent-when {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  /* hero */

  .sp-hero {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--sp-4);
  }

  .sp-hero-main {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-hero-head {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .sp-hero-name {
    margin: 0;
    font-size: var(--sp-fs-xl);
    font-weight: var(--sp-fw-bold);
    letter-spacing: -0.01em;
    color: var(--sp-text-1);
  }

  .sp-hero-desc {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
    line-height: var(--sp-lh-normal);
  }

  .sp-hero-path {
    margin: 0;
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    word-break: break-all;
  }

  .sp-hero-tags {
    display: flex;
    gap: var(--sp-1);
    flex-wrap: wrap;
  }

  .sp-hero-opened {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  /* stats */

  .sp-stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
    gap: var(--sp-3);
    margin: var(--sp-4) 0;
  }

  .sp-stat {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-4);
    background: var(--sp-glass-bg);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    box-shadow: var(--sp-shadow-1);
  }

  .sp-stat-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2.25rem;
    height: 2.25rem;
    border-radius: var(--sp-radius-md);
    flex-shrink: 0;
  }

  .sp-stat-icon-lime {
    color: var(--sp-success);
    background: rgba(163, 230, 53, 0.12);
    border: 1px solid rgba(163, 230, 53, 0.3);
  }

  .sp-stat-icon-violet {
    color: var(--sp-violet);
    background: rgba(139, 92, 246, 0.12);
    border: 1px solid rgba(139, 92, 246, 0.3);
  }

  .sp-stat-icon-amber {
    color: var(--sp-warning);
    background: rgba(251, 191, 36, 0.12);
    border: 1px solid rgba(251, 191, 36, 0.3);
  }

  .sp-stat-icon-cyan {
    color: var(--sp-info);
    background: rgba(34, 211, 238, 0.12);
    border: 1px solid rgba(34, 211, 238, 0.3);
  }

  .sp-stat-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .sp-stat-value {
    font-size: var(--sp-fs-xl);
    font-weight: var(--sp-fw-bold);
    color: var(--sp-text-1);
    line-height: var(--sp-lh-tight);
  }

  .sp-stat-label {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  /* grid */

  .sp-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(20rem, 1fr));
    gap: var(--sp-4);
  }

  .sp-inline-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-6) var(--sp-4);
    text-align: center;
  }

  .sp-inline-empty p {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
  }

  .sp-proc-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-proc-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .sp-proc-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--sp-success);
    flex-shrink: 0;
  }

  .sp-proc-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .sp-proc-label {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sp-proc-meta {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-family: var(--sp-font-mono);
  }

  .sp-proc-warn {
    margin: var(--sp-3) 0 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-warning);
  }

  .sp-session-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--sp-3);
  }

  .sp-session-item {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding: var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .sp-session-label {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .sp-session-value {
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-session-err {
    color: var(--sp-danger);
  }
</style>
