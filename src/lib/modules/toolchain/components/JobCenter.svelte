<script lang="ts">
  // Центр операций: активные задания, история сессии, expandable-логи.
  // Технический вывод — в свёрнутых областях; сырой вывод не главный UI.
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import Progress from "$lib/components/ui/Progress.svelte";
  import { toolchain } from "../state.svelte";
  import {
    formatRelativeTime,
    jobStatusLabel,
    operationLabel,
    phaseLabel,
    sanitizeErrorMessage,
    scanPhaseLabel,
    scanTerminalLabel,
  } from "../format";
  import { engineTaskStatusKind, jobStatusIsTerminal } from "../types";

  let expandedJobId = $state<string | null>(null);

  const history = $derived(toolchain.jobHistory);
  const activeScan = $derived(
    toolchain.currentScan && toolchain.currentScan.running ? toolchain.currentScan : null,
  );
  const activeJob = $derived(
    toolchain.currentJob && !jobStatusIsTerminal(toolchain.currentJob.status)
      ? toolchain.currentJob
      : null,
  );

  function jobDoneTasks(jobId: string): number {
    const job =
      jobId === toolchain.currentJob?.job_id
        ? toolchain.currentJob
        : history.find((j) => j.job_id === jobId);
    if (!job) return 0;
    return job.plan.tasks.filter((t) => {
      const kind = engineTaskStatusKind(t.status);
      return kind !== "pending" && kind !== "running";
    }).length;
  }

  function toggleLog(jobId: string): void {
    expandedJobId = expandedJobId === jobId ? null : jobId;
  }

  async function retry(jobId: string): Promise<void> {
    await toolchain.retryJob(jobId);
  }
</script>

<Card
  title="Операции"
  description="Установки, обновления и сканирования с журналом выполнения"
>
  {#snippet actions()}
    {#if activeScan}
      <Badge tone="cyan" dot>скан</Badge>
    {/if}
    {#if activeJob}
      <Badge tone="violet" dot>{operationLabel(activeJob.operation)}</Badge>
    {/if}
  {/snippet}

  <div class="jobs">
    <!-- Активный скан -->
    {#if activeScan}
      <div class="row">
        <span class="row-icon scan"><Icon name="search" size={15} /></span>
        <div class="row-main">
          <div class="row-title-row">
            <span class="row-title">Диагностический скан</span>
            <Badge tone={activeScan.cancel_requested ? "amber" : "cyan"}>{scanPhaseLabel(activeScan.phase)}</Badge>
            <span class="row-count">{activeScan.completed_tools}/{activeScan.total_tools}</span>
          </div>
          <Progress value={activeScan.completed_tools} max={Math.max(1, activeScan.total_tools)} size="sm" />
        </div>
        <IconButton
          icon="x"
          label="Отменить скан"
          size="sm"
          onclick={() => toolchain.cancelCurrentScan()}
        />
      </div>
    {:else if toolchain.currentScan && toolchain.currentScan.terminal !== "Running"}
      <div class="row row-done">
        <span class="row-icon done"><Icon name="check" size={15} /></span>
        <div class="row-main">
          <div class="row-title-row">
            <span class="row-title">Скан {scanTerminalLabel(toolchain.currentScan.terminal)}</span>
            <span class="row-count">{toolchain.currentScan.completed_tools}/{toolchain.currentScan.total_tools}</span>
          </div>
        </div>
        <Button variant="ghost" size="sm" icon="refresh" onclick={() => toolchain.ensureScanRunning()}>
          Ещё раз
        </Button>
      </div>
    {/if}

    <!-- Активное задание -->
    {#if activeJob}
      {@const done = jobDoneTasks(activeJob.job_id)}
      <div class="row">
        <span class="row-icon run"><Icon name="wrench" size={15} /></span>
        <div class="row-main">
          <div class="row-title-row">
            <span class="row-title">{operationLabel(activeJob.operation)}</span>
            <Badge tone={jobStatusLabel(activeJob.status).tone}>
              {jobStatusLabel(activeJob.status).label}
            </Badge>
            <span class="row-id">{activeJob.job_id}</span>
          </div>
          <Progress value={done} max={Math.max(1, activeJob.plan.tasks.length)} size="sm" />
          {#each activeJob.plan.tasks.filter((t) => engineTaskStatusKind(t.status) === "running") as t (t.task_id)}
            <p class="row-phase">
              {t.display}:{" "}
              {typeof t.status === "object" && "running" in t.status
                ? phaseLabel(t.status.running.phase)
                : "выполняется…"}
            </p>
          {/each}
          {#if activeJob.errors.length > 0}
            <p class="row-error">{sanitizeErrorMessage(activeJob.errors[0])}</p>
          {/if}
        </div>
        <IconButton
          icon="x"
          label="Отменить задание"
          size="sm"
          onclick={() => toolchain.cancelCurrentJob()}
        />
      </div>
    {/if}

    <!-- История -->
    {#if history.length === 0 && !activeScan && !activeJob}
      <EmptyState
        compact
        icon="clock"
        title="Операций пока не было"
        description="Установки инструментов, обновления и сканирования появятся здесь вместе с журналом выполнения."
      />
    {:else if history.length > 0}
      <ul class="history" aria-label="История операций">
        {#each history as job (job.job_id)}
          {@const status = jobStatusLabel(job.status)}
          {@const logs = toolchain.jobLogFor(job.job_id)}
          <li class="hrow">
            <div class="hrow-line">
              <button
                type="button"
                class="hrow-toggle"
                onclick={() => toggleLog(job.job_id)}
                aria-expanded={expandedJobId === job.job_id}
              >
                <Badge tone={status.tone}>{status.label}</Badge>
                <span class="hrow-op">{operationLabel(job.operation)}</span>
                <span class="hrow-meta">
                  {job.requested_tool_ids.length > 0
                    ? `${job.requested_tool_ids.length} инстр.`
                    : "без задач"}
                  · {formatRelativeTime(job.finished_at ?? job.updated_at ?? job.created_at)}
                </span>
                {#if job.recovered}<Badge tone="amber">после перезапуска</Badge>{/if}
              </button>
              <div class="hrow-actions">
                {#if job.status === "failed" || job.status === "interrupted" || job.status === "cancelled"}
                  <Button variant="ghost" size="sm" icon="refresh" onclick={() => retry(job.job_id)}>
                    Повторить
                  </Button>
                {/if}
                <IconButton
                  icon={expandedJobId === job.job_id ? "chevronRight" : "terminal"}
                  label={expandedJobId === job.job_id ? "Свернуть журнал" : "Показать журнал"}
                  size="sm"
                  onclick={() => toggleLog(job.job_id)}
                />
              </div>
            </div>
            {#if expandedJobId === job.job_id}
              <div class="logs">
                {#if job.errors.length > 0}
                  <p class="log-errors">
                    {#each job.errors as e}
                      <span>{sanitizeErrorMessage(e)}</span>
                    {/each}
                  </p>
                {/if}
                {#if logs.length > 0}
                  <pre class="log-lines">{#each logs as line}{line.text}
{/each}</pre>
                {:else}
                  <p class="log-empty">
                    Журнал этой операции недоступен (запись восстановлена из предыдущей сессии).
                  </p>
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</Card>

<style>
  .jobs {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-3);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    background: var(--sp-bg-2);
  }

  .row-done {
    opacity: 0.75;
  }

  .row-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.9rem;
    height: 1.9rem;
    flex: 0 0 auto;
    border-radius: var(--sp-radius-md);
    border: 1px solid var(--sp-border);
  }

  .row-icon.scan { color: var(--sp-cyan); }
  .row-icon.run { color: var(--sp-violet); }
  .row-icon.done { color: var(--sp-lime); }

  .row-main {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    flex: 1 1 auto;
    min-width: 0;
  }

  .row-title-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
    flex-wrap: wrap;
  }

  .row-title {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .row-id {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .row-count {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-variant-numeric: tabular-nums;
  }

  .row-phase {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
  }

  .row-error {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-danger);
    word-break: break-word;
  }

  .history {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }

  .hrow {
    border-top: 1px solid var(--sp-border-faint);
    padding: var(--sp-2) var(--sp-1);
  }

  .hrow:first-child {
    border-top: none;
  }

  .hrow-line {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }

  .hrow-toggle {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    flex: 1 1 auto;
    min-width: 0;
    padding: var(--sp-1) var(--sp-2);
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    border-radius: var(--sp-radius-sm);
  }

  .hrow-toggle:hover {
    background: var(--sp-bg-2);
  }

  .hrow-op {
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-1);
    white-space: nowrap;
  }

  .hrow-meta {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hrow-actions {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    flex: 0 0 auto;
  }

  .logs {
    margin: var(--sp-2) 0 var(--sp-1);
    padding: var(--sp-3);
    background: var(--sp-code-bg);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .log-lines {
    margin: 0;
    max-height: 12rem;
    overflow: auto;
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .log-errors {
    margin: 0 0 var(--sp-2);
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    font-size: var(--sp-fs-xs);
    color: var(--sp-danger);
  }

  .log-empty {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-style: italic;
  }
</style>
