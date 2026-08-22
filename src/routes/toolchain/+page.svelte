<script lang="ts">
  // ================================================================
  // Toolchain Control Center — канонический маршрут /toolchain.
  //
  // Вход на страницу:
  //   1. кэшированный снапшот рендерится мгновенно;
  //   2. в фоне стартует read-only скан (контракт §4.2);
  //   3. карточки дополняются инкрементально, без стирания данных;
  //   4. провал скана не блокирует страницу и предлагает повтор.
  // ================================================================
  import { onMount } from "svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Tabs from "$lib/components/ui/Tabs.svelte";
  import { toolchain } from "$lib/modules/toolchain/state.svelte";
  import type { CardPlanOp } from "$lib/modules/toolchain/components/ToolCard.svelte";
  import ActivityStrip from "$lib/modules/toolchain/components/ActivityStrip.svelte";
  import EnvironmentHero from "$lib/modules/toolchain/components/EnvironmentHero.svelte";
  import JobCenter from "$lib/modules/toolchain/components/JobCenter.svelte";
  import ToolDetailDrawer from "$lib/modules/toolchain/components/ToolDetailDrawer.svelte";
  import BuildEnvironmentMode from "$lib/modules/toolchain/components/BuildEnvironmentMode.svelte";
  import ManageEverythingMode from "$lib/modules/toolchain/components/ManageEverythingMode.svelte";
  import MarketplaceMode from "$lib/modules/toolchain/components/MarketplaceMode.svelte";
  import PlanReviewModal, {
    type PlanRequest,
  } from "$lib/modules/toolchain/components/PlanReviewModal.svelte";
  import {
    formatAgeSeconds,
    platformName,
  } from "$lib/modules/toolchain/format";
  import { defaultCatalogFilters } from "$lib/modules/toolchain/filters";

  let planRequest = $state<PlanRequest | null>(null);
  let busyRecheck = $state(false);

  const scanning = $derived(!!toolchain.currentScan && toolchain.currentScan.running);
  const snapshot = $derived(toolchain.liveSnapshot);
  const freshness = $derived(toolchain.freshness);

  const selectedToolId = $derived(toolchain.selectedToolId);
  const drawerOpen = $derived(toolchain.drawerOpen);
  const selectedTool = $derived(
    selectedToolId ? toolchain.toolById(selectedToolId) : null,
  );
  const selectedDef = $derived(selectedToolId ? toolchain.definitionFor(selectedToolId) : null);

  /** Живые состояния из снапшота по id (для статусов профиля). */
  const liveStateById = $derived(
    Object.fromEntries((snapshot?.tools ?? []).map((t) => [t.tool_id, t])),
  );

  /** Инструменты каталога, зависящие от выбранного (обратные связи). */
  const dependents = $derived.by(() => {
    if (!selectedToolId) return [];
    const out: string[] = [];
    for (const def of Object.values(toolchain.definitions)) {
      if (
        def.bundled_with === selectedToolId ||
        (def.dependencies ?? []).includes(selectedToolId)
      ) {
        out.push(def.id);
      }
    }
    return out;
  });

  /** Журналы заданий, относящиеся к выбранному инструменту. */
  const toolLogLines = $derived.by(() => {
    if (!selectedToolId) return [];
    const lines: { text: string; timestamp: string }[] = [];
    for (const entries of Object.values(toolchain.jobLogs)) {
      for (const e of entries) {
        if (e.tool_id === selectedToolId) lines.push({ text: e.text, timestamp: e.timestamp });
      }
    }
    return lines.slice(-100);
  });

  const modeTabs = [
    { id: "manage_everything", label: "Управлять всем", icon: "layers" as const },
    { id: "build_environment", label: "Собрать окружение", icon: "sparkles" as const },
    { id: "tool_marketplace", label: "Витрина инструментов", icon: "store" as const },
  ];

  onMount(() => {
    const cleanup = toolchain.ensureInitialized();
    // Автоскан при входе (§4.2): reconnect или новый read-only запуск.
    void toolchain.ensureScanRunning();
    return cleanup;
  });

  // Живое обновление деталей при открытии drawer'а.
  $effect(() => {
    if (drawerOpen && selectedToolId) void toolchain.refreshToolDetails(selectedToolId);
  });

  function switchMode(id: string): void {
    if (id === "build_environment") toolchain.setMode("build_environment");
    else if (id === "tool_marketplace") toolchain.setMode("tool_marketplace");
    else toolchain.setMode("manage_everything");
  }

  function reviewUpdates(): void {
    toolchain.setMode("manage_everything");
    // Обзор обновлений = только фильтр update_only; все прочие группы
    // сбрасываются (иначе «обзор» молча скрывал бы инструменты).
    toolchain.setFilters({ ...defaultCatalogFilters(), update_only: true });
  }

  async function recheck(toolId: string): Promise<void> {
    busyRecheck = true;
    try {
      await toolchain.runHealthChecks([toolId]);
    } finally {
      busyRecheck = false;
    }
  }

  function openPlan(operation: CardPlanOp, toolIds: string[]): void {
    planRequest = { operation, toolIds };
  }

  const activeOpLabel = $derived.by(() => {
    if (scanning && toolchain.currentScan) {
      return `скан ${toolchain.currentScan.completed_tools}/${toolchain.currentScan.total_tools}`;
    }
    const job = toolchain.currentJob;
    if (job && ["queued", "running"].includes(job.status)) {
      return job.operation;
    }
    return null;
  });

  const issues = $derived(
    snapshot ? [...snapshot.errors, ...snapshot.warnings].slice(0, 3) : [],
  );
</script>

<PageContainer width="wide">
  <PageHeader
    title="Toolchain"
    description="Центр управления окружением разработки: что установлено, что сломано и чего не хватает"
    icon="wrench"
  >
    {#snippet actions()}
      <div class="header-actions">
        {#if snapshot}
          <Badge tone="neutral">{platformName(snapshot.os)} · {snapshot.arch}</Badge>
          {#if freshness === "live"}
            <Badge tone="cyan" dot>данные актуальны</Badge>
          {:else if freshness === "stale"}
            <Badge tone="amber" dot>устарели · {formatAgeSeconds(snapshot.age_seconds)}</Badge>
          {/if}
        {:else}
          <Badge tone="neutral">нет данных скана</Badge>
        {/if}

        {#if activeOpLabel}
          <button
            type="button"
            class="active-op"
            onclick={() => toolchain.toggleLogPanel(true)}
            title="Открыть центр операций"
          >
            <span class="active-op-dot" aria-hidden="true"></span>
            {activeOpLabel}
          </button>
        {/if}

        <Button
          variant="primary"
          size="sm"
          icon="refresh"
          loading={scanning || toolchain.scanReconnecting}
          onclick={() => void toolchain.ensureScanRunning()}
        >
          {scanning ? "Сканирование…" : toolchain.scanReconnecting ? "Подключение…" : "Сканировать"}
        </Button>
        <IconButton
          icon="terminal"
          label={toolchain.logPanelOpen ? "Скрыть операции" : "Показать операции"}
          variant={toolchain.logPanelOpen ? "solid" : "ghost"}
          onclick={() => toolchain.toggleLogPanel()}
        />
      </div>
    {/snippet}
  </PageHeader>

  <ActivityStrip />

  <!-- ===== Герой состояния окружения ===== -->
  <div class="hero-block">
    <EnvironmentHero
      {snapshot}
      {freshness}
      {scanning}
      onscan={() => void toolchain.ensureScanRunning()}
      onbuild={() => switchMode("build_environment")}
      onupdates={reviewUpdates}
    />
  </div>

  <!-- ===== Проблемы последнего скана (честно, не скрыты) ===== -->
  {#if issues.length > 0}
    <div class="issues" role="alert">
      {#each issues as issue (issue.code + issue.message)}
        <p class={`issue ${issue.code.includes("cancel") ? "" : "issue-warn"}`}>
          <Icon name={issue.code.includes("cancel") ? "info" : "alert"} size={13} />
          <span>{issue.message}</span>
        </p>
      {/each}
    </div>
  {/if}

  <!-- ===== Переключатель режимов ===== -->
  <div class="modes">
    <Tabs tabs={modeTabs} value={toolchain.mode} onchange={switchMode} />
    <span class="modes-hint" id="mode-hint">
      {toolchain.mode === "build_environment"
        ? "Выберите стек — бэкенд соберёт план окружения"
        : toolchain.mode === "tool_marketplace"
          ? "Каталог standalone Toolchain для этой платформы — работает и без скана"
          : "Полный каталог инструментов этой машины"}
    </span>
  </div>

  <!-- ===== Содержимое режима (все ветки живут: выбор не теряется) ===== -->
  <div class="mode-pane" hidden={toolchain.mode !== "build_environment"}>
    <Card padding="md">
      <BuildEnvironmentMode liveStateById={liveStateById} onplan={openPlan} onopenmanage={() => switchMode("manage_everything")} />
    </Card>
  </div>
  <div class="mode-pane" hidden={toolchain.mode !== "manage_everything"}>
    <ManageEverythingMode
      onplan={(op, id) => openPlan(op, [id])}
      onrecheck={(id) => recheck(id)}
    />
  </div>
  <div class="mode-pane" hidden={toolchain.mode !== "tool_marketplace"}>
    <MarketplaceMode
      onplan={(op, id) => openPlan(op as CardPlanOp, [id])}
      ondetails={(id) => toolchain.selectTool(id)}
    />
  </div>

  <!-- ===== Центр операций (сворачиваемый) ===== -->
  {#if toolchain.logPanelOpen}
    <div class="job-center">
      <JobCenter />
    </div>
  {/if}

  <!-- ===== Drawer деталей инструмента ===== -->
  <ToolDetailDrawer
    open={drawerOpen}
    tool={selectedTool}
    def={selectedDef}
    dependents={dependents}
    jobLogLines={toolLogLines}
    busyRecheck={busyRecheck}
    busyDetails={toolchain.detailsLoading}
    detailsError={toolchain.detailsError}
    detailsRetry={
      selectedToolId ? () => void toolchain.refreshToolDetails(selectedToolId) : null
    }
    os={snapshot?.os ?? ""}
    adopted={selectedToolId ? toolchain.isAdopted(selectedToolId) : false}
    onclose={() => toolchain.toggleDrawer(false)}
    onplan={(op, id) => {
      toolchain.toggleDrawer(false);
      openPlan(op, [id]);
    }}
    onrecheck={(id) => void recheck(id)}
    onadopt={(id) => void toolchain.adoptTool(id)}
  />

  <!-- ===== План установки ===== -->
  <PlanReviewModal request={planRequest} onclose={() => (planRequest = null)} />
</PageContainer>

<style>
  .header-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }

  .hero-block {
    margin-bottom: var(--sp-5);
  }

  .active-op {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-3);
    border-radius: var(--sp-radius-full);
    border: 1px solid rgba(34, 211, 238, 0.35);
    background: rgba(34, 211, 238, 0.1);
    color: var(--sp-cyan);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
    cursor: pointer;
    white-space: nowrap;
  }

  .active-op:hover {
    background: rgba(34, 211, 238, 0.16);
  }

  .active-op-dot {
    width: 0.45rem;
    height: 0.45rem;
    border-radius: var(--sp-radius-full);
    background: currentColor;
    animation: pulse 1.6s ease-in-out infinite;
  }

  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.35;
    }
  }

  .issues {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    margin-bottom: var(--sp-5);
  }

  .issue {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin: 0;
    padding: var(--sp-2) var(--sp-3);
    font-size: var(--sp-fs-xs);
    color: var(--sp-danger);
    border: 1px solid rgba(248, 113, 113, 0.28);
    background: rgba(248, 113, 113, 0.07);
    border-radius: var(--sp-radius-md);
  }

  .issue-warn {
    color: var(--sp-warning);
    border-color: rgba(251, 191, 36, 0.3);
    background: rgba(251, 191, 36, 0.07);
  }

  .modes {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    flex-wrap: wrap;
    margin-bottom: var(--sp-5);
  }

  .modes-hint {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .mode-pane {
    min-width: 0;
  }

  .mode-pane[hidden] {
    display: none;
  }

  .job-center {
    margin-top: var(--sp-8);
  }
</style>
