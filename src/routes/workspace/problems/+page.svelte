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
  import { statusLabel, formatStarted, isProcessFailed } from "$lib/modules/workspace/status";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

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
      error = i18n.t("devl.load_processes_failed", { err: String(e) });
    }
  }

  // Problems are derived client-side from real process state (the backend
  // get_problems command has no frontend wrapper — contract E).
  const problems = $derived(
    processes.filter((p) => {
      if (isProcessFailed(p.status)) return true;
      if (p.last_error) return true;
      return false;
    }),
  );

  function problemReason(p: TrackedProcess): string {
    if (p.last_error) return p.last_error;
    if (p.status === "crashed") return i18n.t("ws.reason_crashed");
    if (p.status === "timed_out") return i18n.t("devl.status_timed_out");
    if (p.status === "killed") return i18n.t("devl.status_killed");
    if (typeof p.status === "object" && "exited" in p.status) {
      return i18n.t("ws.reason_exited", { code: p.status.exited });
    }
    if (typeof p.status === "object" && "exited_with_error" in p.status) {
      return i18n.t("ws.reason_exited", { code: p.status.exited_with_error });
    }
    return i18n.t("ws.reason_error");
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title={i18n.t("ws.problems") as TranslationKey}
    description={i18n.t("ws.problems_desc") as TranslationKey}
    icon="alert"
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
    <ErrorState title={i18n.t("devl.load_processes_failed") as TranslationKey} message={error} />
  {:else if problems.length === 0}
    <Card variant="elevated" padding="lg">
      <div class="sp-clear">
        <span class="sp-clear-icon" aria-hidden="true">
          <Icon name="check" size={22} />
        </span>
        <div>
          <h3 class="sp-clear-title">{i18n.t("ws.no_problems") as TranslationKey}</h3>
          <p class="sp-clear-desc">
            {i18n.t("ws.no_problems_desc", { n: processes.length }) as TranslationKey}
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
              <span>{i18n.t("ws.pid", { pid: p.pid }) as TranslationKey}</span>
              <span class="sp-sep">·</span>
              <span>{i18n.t("devl.started", { when: formatStarted(p.started_at) }) as TranslationKey}</span>
              {#if p.restarts > 0}
                <span class="sp-sep">·</span>
                <span>{i18n.t("devl.restarts", { n: p.restarts }) as TranslationKey}</span>
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
    background: var(--sp-success-soft);
    border: 1px solid var(--sp-success-border);
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
    background: var(--sp-danger-soft);
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

  .sp-sep {
    color: var(--sp-text-3);
  }

  .sp-problem-error {
    font-size: var(--sp-fs-xs);
    color: var(--sp-danger);
    background: var(--sp-danger-soft);
    border: 1px solid var(--sp-danger-border);
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--sp-radius-sm);
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 6rem;
    overflow-y: auto;
  }
</style>
