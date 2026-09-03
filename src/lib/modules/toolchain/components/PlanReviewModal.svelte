<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  // Экран проверки плана установки/обновления/ремонта PATH.
  // План строит БЭКЕНД из ограниченного запроса (tcx_build_plan);
  // UI показывает задачи и предупреждения и требует явного одобрения.
  //
  // Жизненный цикл запроса плана (конечные состояния, без «вечного
  // строительства»):
  //   validating  — проверка запроса/снапшота;
  //   refreshing  — ТОЧЕЧНАЯ перепроверка выбранных инструментов,
  //                 только если снапшот устарел (никогда не полный скан);
  //   preparing   — бэкенд строит канонический план;
  //   ready       — план получен;
  //   failed      — структурированная ошибка (с повтором).
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
    planTaskIsActionable,
    planTaskIsNoop,
    planWarningInfo,
    sanitizeErrorMessage,
    selectedSourceDescription,
    taskActionLabel,
  } from "../format";
  import type {
    CanonicalPlan,
    DockerCapability,
    OperationKind,
    PlanWarning,
    ToolDefinition,
  } from "../types";
  import { isRecord } from "../types";
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

  /** Различимые состояния построения плана. */
  type PlanPhase =
    | { stage: "validating" }
    | { stage: "refreshing" }
    | { stage: "preparing" }
    | { stage: "ready" }
    | { stage: "failed"; message: string };

  const PHASE_LABELS: Record<Exclude<PlanPhase["stage"], "failed">, TranslationKey> = {
    validating: i18n.t("tc.plan.validating") as TranslationKey,
    refreshing: i18n.t("tc.plan.refreshing") as TranslationKey,
    preparing: i18n.t("tc.plan.preparing") as TranslationKey,
    ready: i18n.t("tc.plan.ready") as TranslationKey,
  };

  let plan = $state<CanonicalPlan | null>(null);
  let phase = $state<PlanPhase>({ stage: "validating" });
  let confirmUnverified = $state(false);
  let confirmAdmin = $state(false);
  let starting = $state(false);

  /** Номер поколения запроса плана: ответ старого запроса (модал был
   *  закрыт и открыт заново для другого набора) не перезаписывает
   *  состояние нового (регрессия «план прошлого выбора в новом окне»). */
  let loadGeneration = 0;

  const open = $derived(request !== null);
  const loading = $derived(
    phase.stage === "validating" ||
      phase.stage === "refreshing" ||
      phase.stage === "preparing",
  );

  /** Подпись текущего этапа загрузки (безопасно для любых состояний). */
  function loadingLabel(stage: PlanPhase["stage"]): string {
    return (PHASE_LABELS as Record<string, TranslationKey | undefined>)[stage] ?? (i18n.t("tc.plan.validating") as TranslationKey);
  }

  /** Отпечаток одобренного превью — уезжает на бэкенд при старте:
   * расхождение отклоняется структурированной ошибкой PlanChanged. */
  const approvedFingerprint = $derived(plan ? plan.fingerprint : null);

  // Границы IPC: предупреждения и задачи разбираются рантайм-guard'ами —
  // малиформированный payload даёт «неизвестное», а не краш `in undefined`.
  // Подсчёты идут по МАШИНОЧИТАЕМОМУ kind (не по тексту подписи):
  // изменение текста предупреждения не ломает гейтинг подтверждений.
  const unverifiedCount = $derived(
    plan ? plan.warnings.filter((w) => planWarningInfo(w).kind === "unverified_source").length : 0,
  );
  const adminTools = $derived(
    plan ? plan.warnings.filter((w) => planWarningInfo(w).kind === "admin_required").length : 0,
  );
  const actionableTasks = $derived(
    plan ? plan.tasks.filter(planTaskIsActionable) : [],
  );
  /** Правдивые no-op задачи (причина показывается в строке задачи). */
  const noopTasks = $derived(plan ? plan.tasks.filter(planTaskIsNoop) : []);

  const gate = $derived<MutationGate>(
    request ? toolchain.mutationGate(request.operation as OperationKind) : { allowed: true },
  );

  const canStart = $derived(
    !!plan &&
      phase.stage === "ready" &&
      actionableTasks.length > 0 &&
      gate.allowed &&
      (unverifiedCount === 0 || confirmUnverified) &&
      (adminTools === 0 || confirmAdmin),
  );

  const blockedReason = $derived.by(() => {
    if (!plan) return null;
    if (actionableTasks.length === 0) {
      return noopTasks.length > 0
        ? (i18n.t("tc.modal.no_actions") as TranslationKey)
        : (i18n.t("tc.modal.no_tasks") as TranslationKey);
    }
    if (!plan.enough_space) {
      return i18n.t("tc.modal.low_disk", { need: formatSizeMb(plan.total_size_mb), free: formatSizeMb(plan.free_space_mb) }) as TranslationKey;
    }
    if (plan.needs_admin_any && plan.capabilities && !plan.capabilities.elevation_supported) {
      return i18n.t("tc.modal.admin_no_support") as TranslationKey;
    }
    if (!gate.allowed) return gate.reason;
    return null;
  });

  // Загрузка превью при открытии / смене запроса.
  $effect(() => {
    if (!request) {
      plan = null;
      phase = { stage: "validating" };
      confirmUnverified = false;
      confirmAdmin = false;
      return;
    }
    void loadPlan();
  });

  /**
   * Планировочный поток БЕЗ полного скана каталога:
   *   1. валидация выбора (локально);
   *   2. последний валидный снапшот используется как есть, пока свеж;
   *   3. устаревший снапшот → ТОЧЕЧНАЯ перепроверка только выбранных
   *      инструментов (ограниченный параллелизм на бэкенде);
   *   4. канонический план строит бэкенд из свежих фактов.
   *
   * Конечность: полёт ограничен сверху таймаутом — «вечного
   * строительства плана» нет (бэкенд уже держит свой 45с-предел,
   * здесь — страховка на сеть/медленный хост).
   */
  const PLAN_FLIGHT_TIMEOUT_MS = 90_000;

  async function loadPlan(): Promise<void> {
    if (!request) return;
    const generation = ++loadGeneration;
    if (request.toolIds.length === 0) {
      plan = null;
      phase = { stage: "failed", message: i18n.t("tc.plan.no_tools_selected") as TranslationKey };
      return;
    }

    phase = { stage: "validating" };

    try {
      // Снапшот читается мгновенно; «stale» решает только объём
      // перепроверки — никогда не запускает скан всего каталога.
      const snapshot = toolchain.liveSnapshot;
      const fresh =
        !!snapshot &&
        toolchain.freshness !== "none" &&
        !snapshot.stale;

      if (!fresh) {
        phase = { stage: "refreshing" };
        // Точечная перепроверка — НЕ фатальный шаг: она лишь освежает
        // карточки. План строится бэкендом по СВЕЖЕМУ обнаружению в любом
        // случае, поэтому сбой перепроверки не должен блокировать план
        // (страховка на случай будущих изменений runHealthChecks).
        try {
          await Promise.race([
            toolchain.runHealthChecks([...request.toolIds]),
            new Promise((_, reject) => setTimeout(() => reject(new Error("Таймаут точечной проверки")), 15000))
          ]);
        } catch {
          /* не фатально: план строится дальше */
        }
      }
      if (generation !== loadGeneration) return;

      phase = { stage: "preparing" };
      const planPromise = api.buildCanonicalPlan(
        // Превью: подтверждения (источники без суммы/UAC) показываются
        // чекбоксами, а не блокируют построение плана ошибкой.
        api.mutationRequest(request.operation as OperationKind, request.toolIds, {
          preview: true,
        }),
      );
      const built = await Promise.race([
        planPromise,
        new Promise<never>((_, reject) =>
          setTimeout(
            () => reject(new Error(i18n.t("tc.plan.timeout_error", { n: PLAN_FLIGHT_TIMEOUT_MS / 1000 }) as TranslationKey)),
            PLAN_FLIGHT_TIMEOUT_MS,
          ),
        ),
      ]);
      if (generation !== loadGeneration) return;
      plan = built;
      phase = { stage: "ready" };
    } catch (err) {
      if (generation !== loadGeneration) return;
      plan = null;
      phase = {
        stage: "failed",
        message: sanitizeErrorMessage(err),
      };
    }
  }

  function definitionFor(toolId: unknown): ToolDefinition | null {
    if (typeof toolId !== "string") return null;
    return toolchain.definitionFor(toolId);
  }

  /** Docker-альтернатива из метаданных каталога: рекомендация отдельно
   * от задачи, никогда не режим исполнения. */
  function dockerInfo(def: ToolDefinition | null): DockerCapability | null {
    const raw = def?.docker;
    return isRecord(raw) ? (raw as DockerCapability) : null;
  }

  function warningsList(warnings: PlanWarning[]): { tone: "amber" | "red"; text: string }[] {
    // Тотальная функция format.planWarningInfo переживает любые payload'ы.
    return warnings.map(planWarningInfo);
  }

  async function start(): Promise<void> {
    if (!request || !canStart || starting) return;
    starting = true;
    const jobId = await toolchain.startMutation(
      api.mutationRequest(request.operation as OperationKind, request.toolIds, {
        confirm_unverified_sources: unverifiedCount > 0 ? confirmUnverified : false,
        confirm_admin_elevation: adminTools > 0 ? confirmAdmin : false,
        expected_plan_fingerprint: approvedFingerprint,
      }),
    );
    starting = false;
    if (jobId) {
      // Прогресс виден сразу: панель операций открывается поверх страницы.
      toolchain.toggleLogPanel(true);
      onclose();
    } else {
      phase = {
        stage: "failed",
        message: i18n.t("tc.plan.start_failed") as TranslationKey,
      };
    }
  }
</script>

<Modal
  {open}
  {onclose}
  title={request
    ? request.operation === "install"
      ? (i18n.t("tc.plan.title_install", { n: request.toolIds.length }) as TranslationKey)
      : request.operation === "update"
        ? (i18n.t("tc.plan.title_update", { n: request.toolIds.length }) as TranslationKey)
        : (i18n.t("tc.plan.title_repair", { n: request.toolIds.length }) as TranslationKey)
    : (i18n.t("tc.plan.title_default") as TranslationKey)}
  description={i18n.t("tc.plan.ready") as TranslationKey}
  size="lg"
>
  {#if loading}
    <LoadingState label={loadingLabel(phase.stage)} />
  {:else if phase.stage === "failed"}
    <ErrorState title={i18n.t("tc.plan.not_built") as TranslationKey} message={phase.message} retry={() => void loadPlan()} />
  {:else if plan}
    <div class="plan">
      <!-- Сводка -->
      <div class="summary">
        <span class="sum-item">
          {i18n.t("tc.plan.tasks_to_run") as TranslationKey} <strong>{actionableTasks.length}</strong>
        </span>
        {#if noopTasks.length > 0}
          <span class="sum-item">
            {i18n.t("tc.plan.no_action") as TranslationKey} <strong>{noopTasks.length}</strong>
          </span>
        {/if}
        <span class="sum-item">
          {i18n.t("tc.plan.total_size") as TranslationKey} <strong>{formatSizeMb(plan.total_size_mb)}</strong>
        </span>
        <span class="sum-item">
          {i18n.t("tc.plan.free_space") as TranslationKey} <strong class={plan.enough_space ? "" : "bad"}>{formatSizeMb(plan.free_space_mb)}</strong>
        </span>
        <Badge tone={plan.enough_space ? "lime" : "red"}>
          {plan.enough_space ? (i18n.t("tc.plan.space_ok") as TranslationKey) : (i18n.t("tc.plan.space_low") as TranslationKey)}
        </Badge>
        {#if plan.needs_admin_any}
          <Badge tone="amber">{i18n.t("tc.plan.admin_needed") as TranslationKey}</Badge>
        {/if}
      </div>

      <!-- Задачи -->
      <ul class="tasks" aria-label={i18n.t("tc.plan.tasks_label") as TranslationKey}>
        {#each plan.tasks as task (task.task_id)}
          {@const isNoop = planTaskIsNoop(task)}
          {@const taskDef = definitionFor(task?.tool_id)}
          {@const docker = dockerInfo(taskDef)}
          <li class="task" class:noop={isNoop}>
            <TechIcon icon={task?.icon} alt="" size="sm" />
            <div class="task-main">
              <span class="task-name">{task?.display ?? task?.tool_id ?? (i18n.t("tc.plan.unknown_task") as TranslationKey)}</span>
              <!-- Причина no-op честна и всегда видна (тотальный форматтер). -->
              <span class="task-action">{taskActionLabel(task)}</span>
              {#if task?.source}
                <span class="task-source">{selectedSourceDescription(task.source)}</span>
              {/if}
              <!-- Локальная установка инфраструктурных тулов — явный факт.
                   Правда по execution_mode задачи (не «у тула есть docker»):
                   хост-задача, у которой есть docker-альтернатива в каталоге. -->
              {#if !isNoop && task?.execution_mode === "host" && taskDef?.docker}
                <span class="task-host">{i18n.t("tc.plan.local_install") as TranslationKey}</span>
              {/if}
              {#if Array.isArray(task?.depends_on) && task.depends_on.length > 0}
                <span class="task-deps">{i18n.t("tc.plan.after_deps", { deps: task.depends_on.join(", ") }) as TranslationKey}</span>
              {/if}
              <!-- Docker — отдельная рекомендация, не действие задачи. -->
              {#if docker}
                <span class="task-docker">
                  <Icon name="info" size={12} />
                  {i18n.t("tc.plan.docker_alt") as TranslationKey}{docker.image ? `: ${docker.image}` : ""}{docker.notes
                    ? ` — ${i18n.t(docker.notes as TranslationKey)}`
                    : ""}
                </span>
              {/if}
            </div>
            <div class="task-side">
              <span class="task-size">{formatSizeMb(task?.size_mb)}</span>
              {#if task?.needs_admin}
                <Badge tone="amber">UAC</Badge>
              {/if}
              {#if isNoop}
                <Badge tone="neutral">{i18n.t("tc.plan.no_action_badge") as TranslationKey}</Badge>
              {/if}
            </div>
          </li>
        {/each}
      </ul>

      <!-- Предупреждения -->
      {#if Array.isArray(plan.warnings) && plan.warnings.length > 0}
        <ul class="warnings" aria-label={i18n.t("tc.plan.warnings_label") as TranslationKey}>
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
            {i18n.t("tc.plan.confirm_unverified", { n: unverifiedCount }) as TranslationKey}
          </span>
        </label>
      {/if}
      {#if adminTools > 0}
        <label class="confirm">
          <input type="checkbox" bind:checked={confirmAdmin} />
          <span>
            {i18n.t("tc.plan.confirm_admin", { n: adminTools }) as TranslationKey}
          </span>
        </label>
      {/if}

      {#if blockedReason}
        <p class="blocked" role="alert">{blockedReason}</p>
      {/if}
    </div>
  {/if}

  {#snippet footer()}
    <Button variant="ghost" onclick={onclose}>{i18n.t("tc.plan.cancel") as TranslationKey}</Button>
    <Button
      variant="primary"
      icon="play"
      disabled={!canStart}
      loading={starting}
      onclick={start}
    >
      {i18n.t("tc.plan.start") as TranslationKey}
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

  .task-host {
    font-size: var(--sp-fs-xs);
    color: var(--sp-lime, var(--sp-accent));
  }

  .task-deps {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-style: italic;
  }

  .task-docker {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
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
    background: rgba(245, 158, 11, 0.08);
    border: 1px solid rgba(245, 158, 11, 0.3);
  }

  .warning-red {
    color: var(--sp-danger);
    background: rgba(239, 68, 68, 0.08);
    border: 1px solid rgba(239, 68, 68, 0.3);
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
    background: rgba(239, 68, 68, 0.08);
    border: 1px solid rgba(239, 68, 68, 0.28);
    border-radius: var(--sp-radius-md);
  }
</style>
