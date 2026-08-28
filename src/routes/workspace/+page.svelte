<script lang="ts">
  import { onDestroy } from "svelte";
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
    isProcessRunning,
    isProcessFailed,
  } from "$lib/modules/workspace/status";
  import { openProject } from "$lib/core/integration";
  import { recentProjects } from "$lib/core/recent";
  import type { RecentProjectRef } from "$lib/core/recent";
  import { notifySuccess, notifyError } from "$lib/core/toasts";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let processes = $state<TrackedProcess[]>([]);
  let session = $state<SessionInfo | null>(null);
  let dataLoaded = $state(false);
  let openingPath = $state<string | null>(null);
  let pollId: ReturnType<typeof setInterval> | null = null;

  const project = $derived($workspaceContext.project);
  const wsLoading = $derived($workspaceContext.loading);
  const wsError = $derived($workspaceContext.error);

  $effect(() => {
    if (project && !dataLoaded) {
      dataLoaded = true;
      loadData();
      startPolling();
    } else if (!project) {
      dataLoaded = false;
      processes = [];
      session = null;
      stopPolling();
    }
  });

  onDestroy(() => {
    stopPolling();
  });

  /** Живое обновление: таймеры (процессы, сессия) считаются на бэкенде,
   *  поэтому страница опрашивает их, пока открыта. Раз в 2 секунды —
   *  статусы процессов и длительность сессии обновляются без перезахода. */
  function startPolling() {
    if (pollId) return;
    pollId = setInterval(() => {
      loadData();
    }, 2000);
  }

  function stopPolling() {
    if (pollId) {
      clearInterval(pollId);
      pollId = null;
    }
  }

  async function loadData() {
    const [procs, sess] = await Promise.all([
      listProcesses().catch(() => [] as TrackedProcess[]),
      getSessionInfo().catch(() => null),
    ]);
    processes = procs;
    session = sess;
  }

  const runningProcs = $derived(processes.filter((p) => isProcessRunning(p.status)));
  const runningCount = $derived(runningProcs.length);
  const erroredCount = $derived(processes.filter((p) => isProcessFailed(p.status)).length);
  const restarts = $derived(processes.reduce((sum, p) => sum + p.restarts, 0));

  async function openRecent(ref: RecentProjectRef) {
    openingPath = ref.path;
    try {
      await openProject(ref.path);
      await reloadWorkspaceContext();
      notifySuccess(i18n.t("ws.toast_project_opened"), ref.name);
    } catch (e) {
      notifyError(
        i18n.t("ws.toast_project_opened"),
        i18n.t("ws.toast_open_failed", { path: ref.path, err: String(e) }),
      );
    }
    openingPath = null;
  }

  function formatWhen(iso: string): string {
    if (!iso) return "";
    const ms = Date.now() - Date.parse(iso);
    if (!Number.isFinite(ms)) return "";
    const mins = Math.floor(ms / 60000);
    if (mins < 1) return i18n.t("time.just_now");
    if (mins < 60) return i18n.t("time.mins_ago", { n: mins });
    const hours = Math.floor(mins / 60);
    if (hours < 24) return i18n.t("time.hours_ago", { n: hours });
    return i18n.t("time.days_ago", { n: Math.floor(hours / 24) });
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
    <LoadingState label={i18n.t("ws.loading_workspace") as TranslationKey} />
  {:else if wsError}
    <ErrorState
      title={i18n.t("ws.load_failed") as TranslationKey}
      message={wsError}
      retry={reloadAll}
    />
  {:else if !project}
    <div class="sp-empty-wrap">
      {#snippet emptyAction()}
        <Button variant="primary" icon="layers" href="/devlauncher">
          {i18n.t("ws.open_devlauncher") as TranslationKey}
        </Button>
        <Button variant="secondary" icon="sparkles" href="/create">
          {i18n.t("ws.open_project_creator") as TranslationKey}
        </Button>
      {/snippet}
      <EmptyState
        icon="folder"
        title={i18n.t("ws.no_project_open") as TranslationKey}
        description={i18n.t("ws.no_project_desc") as TranslationKey}
        action={emptyAction}
      />
    </div>

    {#if $recentProjects.length > 0}
      <div class="sp-recent-section">
        <Card
          title={i18n.t("ws.recent_projects") as TranslationKey}
          description={i18n.t("ws.recent_desc") as TranslationKey}
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
                  {i18n.t("ws.open") as TranslationKey}
                </Button>
              </div>
            {/each}
          </div>
        </Card>
      </div>
    {/if}
  {:else}
    <PageHeader
      title={i18n.t("ws.overview_title") as TranslationKey}
      description={i18n.t("ws.overview_desc") as TranslationKey}
      icon="layers"
    />

    <Card variant="elevated" padding="lg">
      <div class="sp-hero">
        <div class="sp-hero-main">
          <div class="sp-hero-head">
            <h2 class="sp-hero-name">{project.profile_name}</h2>
            <Badge tone="violet">{i18n.t("devl.current") as TranslationKey}</Badge>
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
            {i18n.t("ws.opened", { when: formatDateTime(project.opened_at) }) as TranslationKey}
          </p>
        </div>
        {#if project.project_path}
          <Button
            variant="secondary"
            icon="folder"
            onclick={() => goto("/workspace/files")}
          >
            {i18n.t("ws.browse_files") as TranslationKey}
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
          <span class="sp-stat-label">{i18n.t("ws.stat_running") as TranslationKey}</span>
        </div>
      </div>
      <div class="sp-stat">
        <span class="sp-stat-icon sp-stat-icon-violet" aria-hidden="true">
          <Icon name="terminal" size={16} />
        </span>
        <div class="sp-stat-text">
          <span class="sp-stat-value">{processes.length}</span>
          <span class="sp-stat-label">{i18n.t("ws.stat_total") as TranslationKey}</span>
        </div>
      </div>
      <div class="sp-stat">
        <span class="sp-stat-icon sp-stat-icon-amber" aria-hidden="true">
          <Icon name="refresh" size={16} />
        </span>
        <div class="sp-stat-text">
          <span class="sp-stat-value">{restarts}</span>
          <span class="sp-stat-label">{i18n.t("ws.stat_restarts") as TranslationKey}</span>
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
          <span class="sp-stat-label">{i18n.t("ws.stat_uptime") as TranslationKey}</span>
        </div>
      </div>
    </div>

    <div class="sp-grid">
      <Card
        title={i18n.t("ws.stat_running") as TranslationKey}
        description={i18n.t("ws.overview_desc") as TranslationKey}
      >
        {#if runningProcs.length === 0}
          <div class="sp-inline-empty">
            <p>{i18n.t("ws.no_processes") as TranslationKey}</p>
            <Button
              size="sm"
              variant="secondary"
              icon="terminal"
              onclick={() => goto("/workspace/runtime")}
            >
              {i18n.t("ws.open_runtime") as TranslationKey}
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
                    {i18n.t("ws.pid", { pid: p.pid }) as TranslationKey} · {formatDuration(p.duration_secs)}
                    {#if p.run_id}
                      · <span class="sp-proc-run">run #{p.run_id.slice(0, 8)}</span>
                    {/if}
                    {#if p.command}
                      · <span class="sp-proc-cmd">{p.command}</span>
                    {/if}
                  </span>
                </div>
                <Badge tone={statusTone(p.status)}>{statusLabel(p.status)}</Badge>
              </div>
            {/each}
          </div>
          {#if erroredCount > 0}
            <p class="sp-proc-warn">
              {i18n.t("ws.errored_count", { n: erroredCount }) as TranslationKey}
            </p>
          {/if}
        {/if}
      </Card>

      <Card
        title={i18n.t("ws.session_title") as TranslationKey}
        description={i18n.t("ws.session_desc") as TranslationKey}
      >
        {#if !session}
          <div class="sp-inline-empty">
            <p>{i18n.t("ws.no_session") as TranslationKey}</p>
          </div>
        {:else}
          <div class="sp-session-grid">
            <div class="sp-session-item">
              <span class="sp-session-label">{i18n.t("ws.started") as TranslationKey}</span>
              <span class="sp-session-value">
                {formatDateTime(session.started_at)}
              </span>
            </div>
            <div class="sp-session-item">
              <span class="sp-session-label">{i18n.t("ws.duration") as TranslationKey}</span>
              <span class="sp-session-value">
                {formatDuration(session.duration_secs)}
              </span>
            </div>
            <div class="sp-session-item">
              <span class="sp-session-label">{i18n.t("ws.processes") as TranslationKey}</span>
              <span class="sp-session-value">{session.process_count}</span>
            </div>
            <div class="sp-session-item">
              <span class="sp-session-label">{i18n.t("ws.errors") as TranslationKey}</span>
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

  .sp-proc-run {
    color: var(--sp-violet);
  }

  .sp-proc-cmd {
    color: var(--sp-text-2);
    word-break: break-all;
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
