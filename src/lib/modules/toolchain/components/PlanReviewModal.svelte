<script lang="ts">
  // Экран проверки плана установки/обновления/ремонта PATH.
  // План строит БЭКЕНД из ограниченного запроса (tcx_build_plan);
  // UI показывает задачи и предупреждения и требует явного одобрения.
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import Modal from "$lib/components/ui/Modal.svelte";
  import TechIcon from "$lib/components/TechIcon.svelte";
  import { toolchain } from "../state.svelte";
  import * as api from "../api";
  import {
    formatSizeMb,
    sanitizeErrorMessage,
    selectedSourceDescription,
    taskActionLabel,
  } from "../format";
  import type {
    CanonicalPlan,
    OperationKind,
    PlanWarning,
  } from "../types";
  import type { MutationGate } from "../stateLogic";
  import type { CardPlanOp } from "./ToolCard.svelte";

  export type PlanRequest = { operation: CardPlanOp; toolIds: string[] };

  let {
    request,
    onclose,
  }: {
    request: PlanRequest | null;
    onclose: () => void;
  } = $props();

  let plan = $state<CanonicalPlan | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let confirmUnverified = $state(false);
  let confirmAdmin = $state(false);
  let starting = $state(false);

  const open = $derived(request !== null);

  const unverifiedCount = $derived(
    plan ? plan.warnings.filter((w) => "unverified_source" in w).length : 0,
  );
  const adminTools = $derived(
    plan ? plan.warnings.filter((w) => "admin_required" in w).length : 0,
  );
  const brokenReinstalls = $derived(
    plan ? plan.warnings.filter((w) => "reinstall_on_broken" in w).length : 0,
  );
  const actionableTasks = $derived(
    plan
      ? plan.tasks.filter((t) => typeof t.action === "object" && !("noop" in t.action))
      : [],
  );

  const gate = $derived<MutationGate>(
    request ? toolchain.mutationGate(request.operation) : { allowed: true },
  );

  const canStart = $derived(
    !!plan &&
      actionableTasks.length > 0 &&
      gate.allowed &&
      (unverifiedCount === 0 || confirmUnverified) &&
      (adminTools === 0 || confirmAdmin),
  );

  const blockedReason = $derived.by(() => {
    if (!plan) return null;
    if (actionableTasks.length === 0) {
      return "Все выбранные инструменты уже в порядке или управляются Docker — исполнять нечего.";
    }
    if (!plan.enough_space) {
      return `Недостаточно места на диске установки: нужно ~${formatSizeMb(plan.total_size_mb)}, свободно ${formatSizeMb(plan.free_space_mb)}.`;
    }
    if (plan.needs_admin_any && plan.capabilities && !plan.capabilities.elevation_supported) {
      return "План требует прав администратора, но повышение прав на этой ОС недоступно.";
    }
    if (!gate.allowed) return gate.reason;
    return null;
  });

  // Загрузка превью при открытии / смене запроса.
  $effect(() => {
    if (!request) {
      plan = null;
      error = null;
      confirmUnverified = false;
      confirmAdmin = false;
      return;
    }
    void loadPlan();
  });

  async function loadPlan(): Promise<void> {
    if (!request) return;
    loading = true;
    error = null;
    try {
      plan = await api.buildCanonicalPlan(
        api.mutationRequest(request.operation as OperationKind, request.toolIds),
      );
    } catch (err) {
      error = sanitizeErrorMessage(err);
    } finally {
      loading = false;
    }
  }

  function warningsList(warnings: PlanWarning[]): { tone: "amber" | "red"; text: string }[] {
    return warnings.map((w) => {
      if ("unverified_source" in w) {
        return {
          tone: "amber" as const,
          text: `${w.unverified_source.tool_id}: источник «${w.unverified_source.source_id}» без контрольной суммы — целостность загрузки проверить нельзя.`,
        };
      }
      if ("admin_required" in w) {
        return {
          tone: "amber" as const,
          text: `${w.admin_required.tool_id}: установка потребует повышения прав (UAC).`,
        };
      }
      return {
        tone: "red" as const,
        text: `${w.reinstall_on_broken.tool_id}: инструмент сломан — будет выполнена переустановка.`,
      };
    });
  }

  async function start(): Promise<void> {
    if (!request || !canStart || starting) return;
    starting = true;
    const jobId = await toolchain.startMutation(
      api.mutationRequest(request.operation as OperationKind, request.toolIds, {
        confirm_unverified_sources: unverifiedCount > 0 ? confirmUnverified : false,
        confirm_admin_elevation: adminTools > 0 ? confirmAdmin : false,
      }),
    );
    starting = false;
    if (jobId) onclose();
  }
</script>

<Modal
  {open}
  {onclose}
  title={request
    ? request.operation === "install"
      ? `План установки · ${request.toolIds.length} инстр.`
      : request.operation === "update"
        ? `План обновления · ${request.toolIds.length} инстр.`
        : `План ремонта PATH · ${request.toolIds.length} инстр.`
    : "План"}
  description="Ничего не выполняется до вашего подтверждения. План построен бэкендом из свежих данных."
  size="lg"
>
  {#if loading}
    <LoadingState label="Строим план…" />
  {:else if error}
    <ErrorState title="Не удалось построить план" message={error} retry={() => void loadPlan()} />
  {:else if plan}
    <div class="plan">
      <!-- Сводка -->
      <div class="summary">
        <span class="sum-item">
          задач: <strong>{actionableTasks.length}</strong>
        </span>
        <span class="sum-item">
          объём: <strong>{formatSizeMb(plan.total_size_mb)}</strong>
        </span>
        <span class="sum-item">
          свободно: <strong class={plan.enough_space ? "" : "bad"}>{formatSizeMb(plan.free_space_mb)}</strong>
        </span>
        <Badge tone={plan.enough_space ? "lime" : "red"}>
          {plan.enough_space ? "места достаточно" : "места недостаточно"}
        </Badge>
        {#if plan.needs_admin_any}
          <Badge tone="amber">нужны права администратора</Badge>
        {/if}
      </div>

      <!-- Задачи -->
      <ul class="tasks" aria-label="Задачи плана">
        {#each plan.tasks as task (task.task_id)}
          {@const isNoop = typeof task.action === "string" || "noop" in task.action}
          <li class="task" class:noop={isNoop}>
            <TechIcon icon={task.icon} alt="" size="sm" />
            <div class="task-main">
              <span class="task-name">{task.display}</span>
              <span class="task-action">{taskActionLabel(task)}</span>
              {#if task.source}
                <span class="task-source">{selectedSourceDescription(task.source)}</span>
              {/if}
              {#if task.depends_on.length > 0}
                <span class="task-deps">после: {task.depends_on.join(", ")}</span>
              {/if}
            </div>
            <div class="task-side">
              <span class="task-size">{formatSizeMb(task.size_mb)}</span>
              {#if task.needs_admin}
                <Badge tone="amber">UAC</Badge>
              {/if}
              {#if isNoop}
                <Badge tone="neutral">без действий</Badge>
              {/if}
            </div>
          </li>
        {/each}
      </ul>

      <!-- Предупреждения -->
      {#if plan.warnings.length > 0}
        <ul class="warnings" aria-label="Предупреждения плана">
          {#each warningsList(plan.warnings) as w}
            <li class={`warning warning-${w.tone}`}>
              <Icon name="alert" size={14} />
              <span>{w.text}</span>
            </li>
          {/each}
        </ul>
      {/if}

      <!-- Подтверждения -->
      {#if unverifiedCount > 0}
        <label class="confirm">
          <input type="checkbox" bind:checked={confirmUnverified} />
          <span>
            Я понимаю, что {unverifiedCount} источник(ов) не имеют контрольных сумм, и подтверждаю установку.
          </span>
        </label>
      {/if}
      {#if adminTools > 0}
        <label class="confirm">
          <input type="checkbox" bind:checked={confirmAdmin} />
          <span>
            Подтверждаю повышение прав администратора ({adminTools} задач(и) через UAC).
          </span>
        </label>
      {/if}

      {#if blockedReason}
        <p class="blocked" role="alert">{blockedReason}</p>
      {/if}
    </div>
  {/if}

  {#snippet footer()}
    <Button variant="ghost" onclick={onclose}>Отмена</Button>
    <Button
      variant="primary"
      icon="play"
      disabled={!canStart}
      loading={starting}
      onclick={start}
    >
      Запустить
    </Button>
  {/snippet}
</Modal>

<style>
  .plan {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }

  .summary {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-3);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
  }

  .sum-item strong {
    color: var(--sp-text-1);
    font-variant-numeric: tabular-nums;
  }

  .sum-item strong.bad {
    color: var(--sp-danger);
  }

  .tasks {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }

  .task {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-2) 0;
    border-bottom: 1px solid var(--sp-border-faint);
  }

  .task:last-child {
    border-bottom: none;
  }

  .task.noop {
    opacity: 0.6;
  }

  .task-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1 1 auto;
  }

  .task-name {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .task-action {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
  }

  .task-source {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .task-deps {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-style: italic;
  }

  .task-side {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex: 0 0 auto;
  }

  .task-size {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-variant-numeric: tabular-nums;
  }

  .warnings {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .warning {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-2);
    font-size: var(--sp-fs-xs);
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--sp-radius-md);
  }

  .warning-amber {
    color: var(--sp-amber);
    background: rgba(251, 191, 36, 0.08);
    border: 1px solid rgba(251, 191, 36, 0.3);
  }

  .warning-red {
    color: var(--sp-danger);
    background: rgba(248, 113, 113, 0.08);
    border: 1px solid rgba(248, 113, 113, 0.3);
  }

  .confirm {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-2);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-1);
    cursor: pointer;
    padding: var(--sp-2) 0;
  }

  .confirm input {
    accent-color: var(--sp-accent-strong);
    width: 1rem;
    height: 1rem;
    margin-top: 0.1rem;
  }

  .blocked {
    margin: 0;
    padding: var(--sp-2) var(--sp-3);
    font-size: var(--sp-fs-sm);
    color: var(--sp-danger);
    background: rgba(248, 113, 113, 0.08);
    border: 1px solid rgba(248, 113, 113, 0.28);
    border-radius: var(--sp-radius-md);
  }
</style>
