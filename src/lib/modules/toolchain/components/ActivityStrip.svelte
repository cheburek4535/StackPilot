<script lang="ts">
  // Полоса активных операций: идущий скан и активное задание.
  // Прогресс, фаза, отмена. Терминалы — с итогом и повтором где безопасно.
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import Progress from "$lib/components/ui/Progress.svelte";
  import { toolchain } from "../state.svelte";
  import {
    jobStatusLabel,
    operationLabel,
    phaseLabel,
    sanitizeErrorMessage,
    scanPhaseLabel,
    scanTerminalLabel,
  } from "../format";
  import { engineTaskStatusKind } from "../types";

  let { onopenactivity }: { onopenactivity?: () => void } = $props();

  const scan = $derived(toolchain.currentScan);
  const scanActive = $derived(!!scan && scan.running);
  const job = $derived(toolchain.currentJob);
  const jobActive = $derived(
    !!job && ["queued", "running"].includes(job.status),
  );

  const doneTasks = $derived(
    job
      ? job.plan.tasks.filter((t) => {
          const kind = engineTaskStatusKind(t.status);
          return kind !== "pending" && kind !== "running";
        }).length
      : 0,
  );
  const runningPhase = $derived.by(() => {
    if (!job) return null;
    const running = job.plan.tasks.find(
      (t) => engineTaskStatusKind(t.status) === "running",
    );
    if (!running) return null;
    return typeof running.status === "object" && "running" in running.status
      ? phaseLabel(running.status.running.phase)
      : null;
  });
  const firstError = $derived(job?.errors[0] ? sanitizeErrorMessage(job.errors[0]) : null);

  async function cancelScan() {
    await toolchain.cancelCurrentScan();
  }
</script>

{#if scanActive || jobActive}
  <div class="strip" role="status" aria-live="polite">
    {#if scanActive}
      <div class="op">
        <span class="op-icon spin" aria-hidden="true"><Icon name="refresh" size={14} /></span>
        <div class="op-main">
          <div class="op-title-row">
            <span class="op-title">Сканирование окружения</span>
            <Badge tone="cyan">{scanPhaseLabel(scan!.phase)}</Badge>
            <span class="op-count">{scan!.completed_tools}/{scan!.total_tools}</span>
            {#if scan!.cancel_requested}<span class="muted">отмена…</span>{/if}
          </div>
          <Progress
            value={scan!.completed_tools}
            max={Math.max(1, scan!.total_tools)}
            size="sm"
          />
        </div>
        <IconButton icon="x" label="Отменить сканирование" size="sm" onclick={cancelScan} />
      </div>
    {/if}

    {#if jobActive && job}
      <div class="op">
        <span class="op-icon spin" aria-hidden="true"><Icon name="wrench" size={14} /></span>
        <div class="op-main">
          <div class="op-title-row">
            <span class="op-title">{operationLabel(job.operation)}</span>
            <Badge tone={jobStatusLabel(job.status).tone}>{jobStatusLabel(job.status).label}</Badge>
            <span class="op-count">{doneTasks}/{job.plan.tasks.length}</span>
            {#if runningPhase}<span class="muted">· {runningPhase}</span>{/if}
          </div>
          <Progress
            value={doneTasks}
            max={Math.max(1, job.plan.tasks.length)}
            size="sm"
          />
        </div>
        <IconButton
          icon="x"
          label="Отменить задание"
          size="sm"
          onclick={() => toolchain.cancelCurrentJob()}
        />
      </div>
      {#if firstError}
        <p class="op-error">{firstError}</p>
      {/if}
    {/if}

    {#if onopenactivity}
      <Button variant="ghost" size="sm" onclick={onopenactivity}>Все операции</Button>
    {/if}
  </div>
{:else if scan && scan.terminal !== "Running"}
  <div class="strip strip-terminal" role="status">
    <Badge tone={scan.terminal === "Cancelled" ? "neutral" : scan.terminal === "Completed" ? "lime" : "amber"}>
      Скан {scanTerminalLabel(scan.terminal)}
    </Badge>
    <span class="muted">{scan.completed_tools}/{scan.total_tools} инструментов</span>
    <Button variant="ghost" size="sm" icon="refresh" onclick={() => toolchain.ensureScanRunning()}>
      Повторить скан
    </Button>
  </div>
{/if}

<style>
  .strip {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding: var(--sp-2) var(--sp-4);
    margin-bottom: var(--sp-5);
    border: 1px solid var(--sp-accent-border);
    border-radius: var(--sp-radius-lg);
    background: var(--sp-glass-bg);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    box-shadow: var(--sp-shadow-1);
  }

  .strip-terminal {
    border-color: var(--sp-border);
  }

  .op {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    flex: 1 1 auto;
    min-width: 0;
  }

  .op + .op {
    border-left: 1px solid var(--sp-border);
    padding-left: var(--sp-4);
  }

  .op-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    flex: 0 0 auto;
    border-radius: var(--sp-radius-md);
    color: var(--sp-cyan);
    background: rgba(34, 211, 238, 0.12);
    border: 1px solid rgba(34, 211, 238, 0.3);
  }

  .op-icon.spin :global(svg) {
    animation: sp-spin 1.2s linear infinite;
  }

  .op-main {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    flex: 1 1 auto;
    min-width: 0;
  }

  .op-title-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }

  .op-title {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .op-count {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .muted {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .op-error {
    margin: 0;
    flex-basis: 100%;
    font-size: var(--sp-fs-xs);
    color: var(--sp-danger);
  }

  @media (max-width: 720px) {
    .strip {
      flex-wrap: wrap;
    }
    .op + .op {
      border-left: none;
      padding-left: 0;
      flex-basis: 100%;
    }
  }
</style>
