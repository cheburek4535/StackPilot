<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import Progress from "$lib/components/ui/Progress.svelte";
  import { toolchain } from "../state.svelte";
  import {
    jobStatusLabel,
    operationLabel,
    phaseLabel,
    sanitizeErrorMessage,
  } from "../format";
  import { engineTaskStatusKind, jobStatusIsTerminal } from "../types";

  const activeJob = $derived(toolchain.currentJob);

  function jobDoneTasks(jobId: string): number {
    if (!activeJob || activeJob.job_id !== jobId) return 0;
    return activeJob.plan.tasks.filter((t) => {
      const kind = engineTaskStatusKind(t.status);
      return kind !== "pending" && kind !== "running";
    }).length;
  }
  
  const jobTitle = $derived.by(() => {
    if (!activeJob) return i18n.t("tc.op.health_check") as TranslationKey;
    const op = operationLabel(activeJob.operation);
    if (activeJob.requested_tool_ids.length > 0) {
      const tools = activeJob.requested_tool_ids.map(id => toolchain.definitionFor(id)?.display || id).join(", ");
      return `${op}: ${tools}`;
    }
    return op;
  });
</script>

{#if activeJob}
<Card title={i18n.t("tc.ui.operations") as TranslationKey} description={i18n.t("tc.ui.current_state") as TranslationKey}>
  <div class="jobs">
      
      <div class="row">
        <span class="row-icon run"><Icon name="wrench" size={15} /></span>
        <div class="row-main">
          <div class="row-title-row">
            <span class="row-title">{jobTitle}</span>
            <Badge tone={jobStatusLabel(activeJob.status).tone}>
              {jobStatusLabel(activeJob.status).label}
            </Badge>
          </div>
          
          {#if !jobStatusIsTerminal(activeJob.status)}
            {@const done = jobDoneTasks(activeJob.job_id)}
            <Progress value={done} max={Math.max(1, activeJob.plan.tasks.length)} size="sm" />
            {#each activeJob.plan.tasks.filter((t) => engineTaskStatusKind(t.status) === "running") as t (t.task_id)}
              <p class="row-phase">
                {t.display}:{" "}
                {t.status !== null && typeof t.status === "object" && "running" in t.status
                  ? phaseLabel(t.status.running.phase)
                  : (i18n.t("tc.job.running") as TranslationKey)}
              </p>
            {/each}
          {/if}

          {#if activeJob.errors.length > 0}
            <p class="row-error">{sanitizeErrorMessage(activeJob.errors[0])}</p>
          {/if}
        </div>
        {#if !jobStatusIsTerminal(activeJob.status)}
          <IconButton
            icon="x"
            label={i18n.t("tc.plan.cancel") as TranslationKey}
            size="sm"
            onclick={() => toolchain.cancelCurrentJob()}
          />
        {/if}
      </div>
      
      <div class="logs active-job-logs" style="margin-top: 1rem; border-top: 1px solid var(--sp-border-faint); padding-top: 1rem;">
        <p class="log-title" style="font-size: var(--sp-fs-xs); color: var(--sp-text-2); margin-bottom: 0.5rem;">{i18n.t("tc.ui.output_log") as TranslationKey}</p>
        {#if toolchain.jobLogFor(activeJob.job_id).length > 0}
          <pre class="log-lines" style="max-height: 300px; overflow-y: auto;">{#each toolchain.jobLogFor(activeJob.job_id) as line}{line.text}{/each}</pre>
        {:else}
          <p class="log-empty">{i18n.t("tc.ui.output_log") as TranslationKey}</p>
        {/if}
      </div>
  </div>
</Card>
{/if}

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
  .row-icon.run { color: var(--sp-violet); }
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
  .log-empty {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-style: italic;
  }
</style>
