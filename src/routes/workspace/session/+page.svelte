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
  import { getSessionInfo } from "$lib/modules/workspace/api";
  import type { SessionInfo } from "$lib/modules/workspace/types";
  import { formatDuration, formatDateTime } from "$lib/modules/workspace/status";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let session = $state<SessionInfo | null>(null);
  let dataLoaded = $state(false);
  let error = $state("");
  let pollId: ReturnType<typeof setInterval> | null = null;

  const project = $derived($workspaceContext.project);
  const wsLoading = $derived($workspaceContext.loading);
  const wsError = $derived($workspaceContext.error);

  $effect(() => {
    if (project && !dataLoaded) {
      dataLoaded = true;
      loadSession();
      startPolling();
    } else if (!project) {
      dataLoaded = false;
      session = null;
      stopPolling();
    }
  });

  onDestroy(() => {
    stopPolling();
  });

  /** Сессионный таймер живёт на бэкенде — опрашиваем его, пока страница
   *  открыта, чтобы duration обновлялся и останавливался без перезахода. */
  function startPolling() {
    if (pollId) return;
    pollId = setInterval(() => {
      loadSession();
    }, 2000);
  }

  function stopPolling() {
    if (pollId) {
      clearInterval(pollId);
      pollId = null;
    }
  }

  async function loadSession() {
    try {
      session = await getSessionInfo();
    } catch (e) {
      error = i18n.t("ws.load_failed", { err: String(e) });
    }
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title={i18n.t("ws.session_title") as TranslationKey}
    description={i18n.t("ws.session_desc2") as TranslationKey}
    icon="clock"
  />

  {#if wsLoading}
    <LoadingState label={i18n.t("ws.loading_workspace") as TranslationKey} />
  {:else if wsError}
    <ErrorState title={i18n.t("ws.load_failed") as TranslationKey} message={wsError} />
  {:else if !project}
    <EmptyState
      icon="folder"
      title={i18n.t("ws.no_project_open") as TranslationKey}
      description={i18n.t("ws.no_project_open") as TranslationKey}
    />
  {:else if error}
    <ErrorState title={i18n.t("ws.session_title") as TranslationKey} message={error} />
  {:else if !session}
    <EmptyState
      icon="clock"
      title={i18n.t("ws.no_session2") as TranslationKey}
      description={i18n.t("ws.no_session_desc2") as TranslationKey}
    />
  {:else}
    <div class="sp-session-grid">
      <div class="sp-session-item">
        <span class="sp-session-icon sp-session-icon-cyan" aria-hidden="true">
          <Icon name="clock" size={18} />
        </span>
        <span class="sp-session-label">{i18n.t("ws.started") as TranslationKey}</span>
        <span class="sp-session-value">{formatDateTime(session.started_at)}</span>
      </div>
      <div class="sp-session-item">
        <span class="sp-session-icon sp-session-icon-violet" aria-hidden="true">
          <Icon name="refresh" size={18} />
        </span>
        <span class="sp-session-label">{i18n.t("ws.duration") as TranslationKey}</span>
        <span class="sp-session-value">{formatDuration(session.duration_secs)}</span>
      </div>
      <div class="sp-session-item">
        <span class="sp-session-icon sp-session-icon-blue" aria-hidden="true">
          <Icon name="terminal" size={18} />
        </span>
        <span class="sp-session-label">{i18n.t("ws.processes") as TranslationKey}</span>
        <span class="sp-session-value">{session.process_count}</span>
      </div>
      <div class="sp-session-item">
        <span class="sp-session-icon sp-session-icon-red" aria-hidden="true">
          <Icon name="alert" size={18} />
        </span>
        <span class="sp-session-label">{i18n.t("ws.errors") as TranslationKey}</span>
        <span
          class="sp-session-value"
          class:sp-session-value-err={session.error_count > 0}
        >
          {session.error_count}
        </span>
      </div>
    </div>

    <Card title={i18n.t("ws.project_context") as TranslationKey}>
      <div class="sp-info-list">
        <div class="sp-info-row">
          <span class="sp-info-key">{i18n.t("ws.profile") as TranslationKey}</span>
          <span class="sp-info-val">
            {project.profile_name}
            <Badge tone="violet">{i18n.t("devl.current") as TranslationKey}</Badge>
          </span>
        </div>
        <div class="sp-info-row">
          <span class="sp-info-key">{i18n.t("ws.path") as TranslationKey}</span>
          <span class="sp-info-val sp-info-mono">{project.project_path ?? "—"}</span>
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
          <span class="sp-info-key">{i18n.t("ws.opened2") as TranslationKey}</span>
          <span class="sp-info-val">{formatDateTime(project.opened_at)}</span>
        </div>
      </div>
    </Card>
  {/if}
</PageContainer>

<style>
  .sp-session-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
    gap: var(--sp-3);
    margin-bottom: var(--sp-5);
  }

  .sp-session-item {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--sp-1);
    padding: var(--sp-4);
    background: var(--sp-glass-bg);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    box-shadow: var(--sp-shadow-1);
  }

  .sp-session-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    border-radius: var(--sp-radius-md);
    margin-bottom: var(--sp-1);
  }

  .sp-session-icon-cyan {
    color: var(--sp-info);
    background: rgba(34, 211, 238, 0.12);
    border: 1px solid rgba(34, 211, 238, 0.3);
  }

  .sp-session-icon-violet {
    color: var(--sp-violet);
    background: rgba(139, 92, 246, 0.12);
    border: 1px solid rgba(139, 92, 246, 0.3);
  }

  .sp-session-icon-blue {
    color: var(--sp-blue);
    background: rgba(96, 165, 250, 0.12);
    border: 1px solid rgba(96, 165, 250, 0.3);
  }

  .sp-session-icon-red {
    color: var(--sp-danger);
    background: rgba(248, 113, 113, 0.12);
    border: 1px solid rgba(248, 113, 113, 0.3);
  }

  .sp-session-label {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .sp-session-value {
    font-size: var(--sp-fs-lg);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    line-height: var(--sp-lh-tight);
  }

  .sp-session-value-err {
    color: var(--sp-danger);
  }

  .sp-info-list {
    display: flex;
    flex-direction: column;
  }

  .sp-info-row {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding: var(--sp-3) 0;
    border-bottom: 1px solid var(--sp-border-faint);
  }

  .sp-info-row:last-child {
    border-bottom: none;
  }

  .sp-info-key {
    flex: 0 0 6rem;
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
