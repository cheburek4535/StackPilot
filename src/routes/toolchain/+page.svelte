<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
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
  import type { Component } from "svelte";
  import PlanReviewModal, {
    type PlanRequest,
  } from "$lib/modules/toolchain/components/PlanReviewModal.svelte";
  import {
    formatAgeSeconds,
    platformName,
  } from "$lib/modules/toolchain/format";
  import { defaultCatalogFilters } from "$lib/modules/toolchain/filters";
  import { markHelpDid, HELP, HINT_TOOLCHAIN_WELCOME, HINT_TOOLCHAIN_MODES } from "$lib/core/help";
  import HelpHint from "$lib/components/ui/HelpHint.svelte";

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let ManageEverythingMode = $state<Component<any>>(null as any);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let BuildEnvironmentMode = $state<Component<any>>(null as any);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let MarketplaceMode = $state<Component<any>>(null as any);

  const loaded = $state<Record<string, boolean>>({});

  async function loadMode(id: string): Promise<void> {
    if (loaded[id]) return;
    loaded[id] = true;
    if (id === "manage_everything") {
      const m = await import("$lib/modules/toolchain/components/ManageEverythingMode.svelte");
      ManageEverythingMode = m.default;
    } else if (id === "build_environment") {
      const m = await import("$lib/modules/toolchain/components/BuildEnvironmentMode.svelte");
      BuildEnvironmentMode = m.default;
    } else if (id === "tool_marketplace") {
      const m = await import("$lib/modules/toolchain/components/MarketplaceMode.svelte");
      MarketplaceMode = m.default;
    }
  }

  loadMode(toolchain.mode);

  $effect(() => {
    loadMode(toolchain.mode);
  });

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
    { id: "manage_everything", label: i18n.t("tc.mode.manage"), icon: "layers" as const },
    { id: "build_environment", label: i18n.t("tc.mode.build"), icon: "sparkles" as const },
    { id: "tool_marketplace", label: i18n.t("tc.mode.marketplace"), icon: "store" as const },
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
    markHelpDid(HELP.toolchainExplored);
    if (id === "build_environment") toolchain.setMode("build_environment");
    else if (id === "tool_marketplace") toolchain.setMode("tool_marketplace");
    else toolchain.setMode("manage_everything");
  }

  function reviewUpdates(): void {
    markHelpDid(HELP.toolchainExplored);
    toolchain.setMode("manage_everything");
    // Обзор обновлений = только фильтр update_only; все прочие группы
    // сбрасываются (иначе «обзор» молча скрывал бы инструменты).
    toolchain.setFilters({ ...defaultCatalogFilters(), update_only: true });
  }

  async function recheck(toolId: string): Promise<void> {
    markHelpDid(HELP.toolchainExplored);
    busyRecheck = true;
    try {
      await toolchain.runHealthChecks([toolId]);
    } finally {
      busyRecheck = false;
    }
  }

  function openPlan(operation: CardPlanOp, toolIds: string[]): void {
    markHelpDid(HELP.toolchainExplored);
    planRequest = { operation, toolIds };
  }

  const activeOpLabel = $derived.by(() => {
    if (scanning && toolchain.currentScan) {
      const s = toolchain.currentScan;
      const currentLabel = s.current_tool ? ` - ${s.current_tool}` : "";
      return `${i18n.t("tc.scan_prefix") as TranslationKey} ${s.completed_tools}/${s.total_tools}${currentLabel}`;
    }
    const job = toolchain.currentJob;
    if (job && ["queued", "running"].includes(job.status)) {
      return job.operation;
    }
    return null;
  });

  const issues = $derived(
    snapshot ? [...(snapshot.errors || []), ...(snapshot.warnings || [])].slice(0, 3) : [],
  );

  /** Open a tool's details and remember the toolchain was explored. */
  function openTool(id: string): void {
    markHelpDid(HELP.toolchainExplored);
    toolchain.selectTool(id);
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title="Toolchain"
    description={i18n.t("tc.page_desc") as TranslationKey}
    icon="wrench"
  >
    {#snippet actions()}
      <div class="header-actions">
        {#if snapshot}
          <Badge tone="neutral">{platformName(snapshot.os)} · {snapshot.arch}</Badge>
          {#if freshness === "live"}
            <Badge tone="cyan" dot>{i18n.t("tc.data_live") as TranslationKey}</Badge>
          {:else if freshness === "stale"}
            <Badge tone="amber" dot>{i18n.t("tc.data_stale") as TranslationKey} · {formatAgeSeconds(snapshot.age_seconds)}</Badge>
          {/if}
        {:else}
          <Badge tone="neutral">{i18n.t("tc.no_data") as TranslationKey}</Badge>
        {/if}

        {#if activeOpLabel}
          <button
            type="button"
            class="active-op"
            onclick={() => toolchain.toggleLogPanel(true)}
            title={i18n.t("tc.open_ops") as TranslationKey}
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
          {scanning ? (i18n.t("tc.scanning") as TranslationKey) : toolchain.scanReconnecting ? (i18n.t("tc.connecting") as TranslationKey) : (i18n.t("tc.scan_btn") as TranslationKey)}
        </Button>
        <IconButton
          icon="terminal"
          label={toolchain.logPanelOpen ? (i18n.t("tc.hide_ops") as TranslationKey) : (i18n.t("tc.show_ops") as TranslationKey)}
          variant={toolchain.logPanelOpen ? "solid" : "ghost"}
          onclick={() => toolchain.toggleLogPanel()}
        />
      </div>
    {/snippet}
  </PageHeader>

  <HelpHint
    id={HINT_TOOLCHAIN_WELCOME.id}
    resolvedBy={HINT_TOOLCHAIN_WELCOME.resolvedBy}
    icon="wrench"
    title={i18n.t("help.toolchain.title") as TranslationKey}
    text={i18n.t("help.toolchain.body") as TranslationKey}
  />

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
      {#each issues as issue, i (i + "-" + issue.code)}
        <p class={`issue ${issue.code.includes("cancel") ? "" : "issue-warn"}`}>
          <Icon name={issue.code.includes("cancel") ? "info" : "alert"} size={13} />
          <span>{issue.message}</span>
        </p>
      {/each}
    </div>
  {/if}

  <!-- ===== Центр операций (сворачиваемый) ===== -->
  {#if toolchain.logPanelOpen}
    <div class="job-center">
      <JobCenter />
    </div>
  {/if}

  <!-- ===== Переключатель режимов ===== -->
  <div class="modes">
    <Tabs tabs={modeTabs} value={toolchain.mode} onchange={switchMode} />
    <span class="modes-hint" id="mode-hint">
      {toolchain.mode === "build_environment"
        ? (i18n.t("tc.hint_build") as TranslationKey)
        : toolchain.mode === "tool_marketplace"
          ? (i18n.t("tc.hint_marketplace") as TranslationKey)
          : (i18n.t("tc.hint_manage") as TranslationKey)}
    </span>
  </div>

  <HelpHint
    id={HINT_TOOLCHAIN_MODES.id}
    resolvedBy={HINT_TOOLCHAIN_MODES.resolvedBy}
    variant="info"
    icon="layers"
    title={i18n.t("help.toolchain_modes.title") as TranslationKey}
  >
    <p>{i18n.t("help.toolchain_modes.body") as TranslationKey}</p>
    <p>{i18n.t("help.toolchain_modes.next") as TranslationKey}</p>
  </HelpHint>

  <!-- ===== Содержимое режима (ленивый монтаж через dynamic import) ===== -->
  <div class="mode-pane" hidden={toolchain.mode !== "build_environment"}>
    {#if BuildEnvironmentMode}
      <Card padding="md">
        <BuildEnvironmentMode liveStateById={liveStateById} onplan={openPlan} onopenmanage={() => switchMode("manage_everything")} />
      </Card>
    {/if}
  </div>
  <div class="mode-pane" hidden={toolchain.mode !== "manage_everything"}>
    {#if ManageEverythingMode}
      <ManageEverythingMode
        onplan={(op: CardPlanOp, id: string) => openPlan(op, [id])}
        onrecheck={(id: string) => recheck(id)}
      />
    {/if}
  </div>
  <div class="mode-pane" hidden={toolchain.mode !== "tool_marketplace"}>
    {#if MarketplaceMode}
      <MarketplaceMode
        onplan={(op: string, id: string) => openPlan(op as CardPlanOp, [id])}
        ondetails={(id: string) => openTool(id)}
      />
    {/if}
  </div>



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

  <!-- ===== Примечание о расширении каталога ===== -->
  <p class="grow-note">
    {i18n.t("tc.catalog_grows_note")}
  </p>
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
    margin-bottom: var(--sp-4);
  }

  .active-op {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-3);
    border-radius: var(--sp-radius-full);
    border: 1px solid var(--sp-info-border);
    background: var(--sp-info-soft);
    color: var(--sp-cyan);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
    cursor: pointer;
    white-space: nowrap;
  }

  .active-op:hover {
    background: var(--sp-info-soft);
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
    border: 1px solid var(--sp-danger-border);
    background: var(--sp-danger-soft);
    border-radius: var(--sp-radius-md);
  }

  .issue-warn {
    color: var(--sp-warning);
    border-color: var(--sp-warning-border);
    background: var(--sp-warning-soft);
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
    margin-bottom: var(--sp-5);
  }

  .grow-note {
    margin: var(--sp-6) auto 0;
    padding-top: var(--sp-4);
    text-align: center;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    opacity: 0.55;
  }
</style>
