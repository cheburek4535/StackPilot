<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  // Режим «Витрина инструментов»: определение-driven маркетплейс.
  // Работает БЕЗ снапшота (только каталог tcx_get_catalog); живые факты
  // (состояние/версия/здоровье/происхождение) накладываются из скана,
  // когда он есть. Установка локально — только через канонический
  // экран проверки плана (никакой Docker-опциональности Project Creator).
  import { onMount } from "svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import MarketplaceCard, {
    type MarketplacePlanOp,
  } from "./MarketplaceCard.svelte";
  import { toolchain } from "../state.svelte";
  import {
    applyMarketplaceFilters,
    buildMarketplaceItems,
    marketplaceBooleanCount,
    marketplaceCapabilityCounts,
    marketplaceCategoryCounts,
    marketplaceExecutionCounts,
    marketplaceHealthCounts,
    marketplaceProvenanceCounts,
    marketplaceStateCounts,
    type MarketplaceItem,
  } from "../marketplace";
  import {
    allProvenanceKinds,
    allToolStateKinds,
    capabilityLabel,
  } from "../format";
  import type { ExecutionMode, HealthState } from "../types";
  import { jobStatusIsTerminal } from "../types";

  let {
    onplan,
    ondetails,
  }: {
    onplan: (operation: MarketplacePlanOp, toolId: string) => void;
    ondetails: (toolId: string) => void;
  } = $props();

  let railOpen = $state(false);

  onMount(() => {
    void toolchain.ensureMarketplacePlatform();
  });

  const os = $derived(toolchain.marketplaceOs);
  const snapshot = $derived(toolchain.liveSnapshot);

  const items = $derived(
    os ? buildMarketplaceItems(toolchain.definitions, snapshot, os) : [],
  );
  
  const stateSortWeights: Record<string, number> = {
    missing: 0,
    update_available: 1,
    unhealthy: 2,
    path_broken: 2,
    installed_unhealthy: 2,
    installed_health_unknown: 3,
    installed_healthy: 3,
    built_in_system: 3,
    unsupported_platform: 4,
    docker_managed: 3,
    manual_install: 3,
    scan_pending: 3,
    scan_failed: 3
  };

  function sortWeight(item: MarketplaceItem): number {
    return stateSortWeights[item.scan?.state.kind ?? "missing"] ?? 99;
  }

  const visibleItems = $derived(
    applyMarketplaceFilters(items, toolchain.filters).sort((a, b) => {
      const wa = sortWeight(a);
      const wb = sortWeight(b);
      if (wa !== wb) return wa - wb;
      return a.def.display.localeCompare(b.def.display);
    })
  );


  /** Идёт активная мутация по инструменту (карточка показывает занятость). */
  const busyIds = $derived.by(() => {
    const job = toolchain.currentJob;
    if (!job || jobStatusIsTerminal(job.status)) return new Set<string>();
    return new Set<string>(job.requested_tool_ids);
  });

  const categoryCounts = $derived(marketplaceCategoryCounts(items));
  const stateCounts = $derived(marketplaceStateCounts(items));
  const healthCounts = $derived(marketplaceHealthCounts(items));
  const provenanceCounts = $derived(marketplaceProvenanceCounts(items));

  /** Скана нет — рантайм-факты отсутствуют, их группы не показываются. */
  const hasScanData = $derived(!!snapshot && snapshot.tools.length > 0);

  const HEALTH_OPTIONS: { kind: HealthState["kind"]; label: TranslationKey }[] = [
    { kind: "healthy", label: i18n.t("tc.health.healthy") as TranslationKey },
    { kind: "degraded", label: i18n.t("tc.health.degraded") as TranslationKey },
    { kind: "unhealthy", label: i18n.t("tc.health.unhealthy") as TranslationKey },
    { kind: "not_checked", label: i18n.t("tc.health.not_checked") as TranslationKey },
  ];

  const capabilityOptions = [
    "installable",
    "updatable",
    "health_checkable",
    "removable",
    "repairable",
    "manual_instructions_available",
    "docker_alternative_available",
  ] as const;

  const QUICK_FILTERS: {
    key:
      | "update_only"
      | "admin_only"
      | "manual_only"
      | "installable"
      | "has_docker_alternative";
    label: TranslationKey;
  }[] = [
    { key: "update_only", label: i18n.t("tc.ui.filter.statuses_only") as TranslationKey },
    { key: "admin_only", label: i18n.t("tc.ui.filter.admin_only") as TranslationKey },
    { key: "manual_only", label: i18n.t("tc.ui.filter.manual_only") as TranslationKey },
    { key: "installable", label: i18n.t("tc.ui.filter.installable") as TranslationKey },
    { key: "has_docker_alternative", label: i18n.t("tc.ui.filter.docker_alt") as TranslationKey },
  ];

  const EXECUTION_OPTIONS: { mode: ExecutionMode; label: TranslationKey }[] = [
    { mode: "host", label: i18n.t("tc.ui.exec.host") as TranslationKey },
    { mode: "docker", label: i18n.t("tc.ui.exec.docker") as TranslationKey },
  ];

  function toggleInArray<T>(arr: T[], value: T): T[] {
    return arr.includes(value) ? arr.filter((x) => x !== value) : [...arr, value];
  }
</script>

<div class="market">
  <!-- ===== Панель поиска ===== -->
  <div class="toolbar">
    <div class="search">
      <Icon name="search" size={16} />
      <input
        type="search"
        placeholder={i18n.t("tc.ui.search_placeholder") as TranslationKey}
        value={toolchain.filters.search}
        oninput={(e) => toolchain.setFilters({ search: e.currentTarget.value })}
        aria-label={i18n.t("tc.ui.search_aria") as TranslationKey}
      />
      {#if toolchain.filters.search}
        <IconButton
          icon="x"
          label={i18n.t("tc.ui.clear_search") as TranslationKey}
          size="sm"
          onclick={() => toolchain.setFilters({ search: "" })}
        />
      {/if}
    </div>

    <div class="rail-toggle-wrap">
      <Button variant="ghost" size="sm" icon={railOpen ? "x" : "layers"} onclick={() => (railOpen = !railOpen)}>
        {i18n.t("tc.ui.filters") as TranslationKey}
      </Button>
    </div>
  </div>

  <div class="content">
    <!-- ===== Рейло фильтров (все значения — с живыми счётчиками) ===== -->
    <aside class="rail" class:rail-open={railOpen} aria-label={i18n.t("tc.ui.catalog_filters") as TranslationKey}>
      <div class="rail-head">
        <span>{i18n.t("tc.ui.filters") as TranslationKey}</span>
        <Button variant="ghost" size="sm" onclick={() => toolchain.resetFilters()}>
          {i18n.t("tc.ui.reset_all") as TranslationKey}
        </Button>
      </div>

      <div class="rail-body">
        {#if !hasScanData}
          <p class="rail-note" role="status">
            <Icon name="info" size={13} />
            {i18n.t("tc.market.no_data") as TranslationKey}
          </p>
        {/if}

        {#if hasScanData}
          <fieldset>
            <legend>{i18n.t("tc.ui.status") as TranslationKey}</legend>
            {#each allToolStateKinds() as { kind, info } (kind)}
              {@const count = stateCounts.get(kind) ?? 0}
              <label class="filter-row" class:filter-disabled={count === 0}>
                <input
                  type="checkbox"
                  disabled={count === 0}
                  checked={toolchain.filters.states.includes(kind)}
                  onchange={() =>
                    toolchain.setFilters({
                      states: toggleInArray(toolchain.filters.states, kind),
                    })}
                />
                <span class="filter-label">{info.label}</span>
                <span class="filter-count">{count}</span>
              </label>
            {/each}
          </fieldset>

          <fieldset>
            <legend>{i18n.t("tc.ui.health") as TranslationKey}</legend>
            {#each HEALTH_OPTIONS as opt (opt.kind)}
              {@const count = healthCounts.get(opt.kind) ?? 0}
              <label class="filter-row" class:filter-disabled={count === 0}>
                <input
                  type="checkbox"
                  disabled={count === 0}
                  checked={toolchain.filters.health.includes(opt.kind)}
                  onchange={() =>
                    toolchain.setFilters({
                      health: toggleInArray(toolchain.filters.health, opt.kind),
                    })}
                />
                <span class="filter-label">{opt.label}</span>
                <span class="filter-count">{count}</span>
              </label>
            {/each}
          </fieldset>

          <fieldset>
            <legend>{i18n.t("tc.ui.origin") as TranslationKey}</legend>
            {#each allProvenanceKinds() as { kind, info } (kind)}
              {@const count = provenanceCounts.get(kind) ?? 0}
              <label class="filter-row" class:filter-disabled={count === 0}>
                <input
                  type="checkbox"
                  disabled={count === 0}
                  checked={toolchain.filters.provenance.includes(kind)}
                  onchange={() =>
                    toolchain.setFilters({
                      provenance: toggleInArray(toolchain.filters.provenance, kind),
                    })}
                />
                <span class="filter-label">{info.label}</span>
                <span class="filter-count">{count}</span>
              </label>
            {/each}
          </fieldset>
        {/if}

        <fieldset>
          <legend>{i18n.t("tc.ui.categories") as TranslationKey}</legend>
          {#each [...categoryCounts.entries()] as [cat, count] (cat)}
            <label class="filter-row" class:filter-disabled={count === 0}>
              <input
                type="checkbox"
                disabled={count === 0}
                checked={toolchain.filters.categories.includes(cat)}
                onchange={() =>
                  toolchain.setFilters({
                    categories: toggleInArray(toolchain.filters.categories, cat),
                  })}
              />
              <span class="filter-label">{cat}</span>
              <span class="filter-count">{count}</span>
            </label>
          {/each}
        </fieldset>

        <fieldset>
          <legend>{i18n.t("tc.ui.capabilities") as TranslationKey}</legend>
          {#each capabilityOptions as flag (flag)}
            {@const count = marketplaceCapabilityCounts(items, flag)}
            <label class="filter-row" class:filter-disabled={count === 0}>
              <input
                type="checkbox"
                disabled={count === 0}
                checked={toolchain.filters.capabilities.includes(flag)}
                onchange={() =>
                  toolchain.setFilters({
                    capabilities: toggleInArray(toolchain.filters.capabilities, flag),
                  })}
              />
              <span class="filter-label">{capabilityLabel(flag)}</span>
              <span class="filter-count">{count}</span>
            </label>
          {/each}
        </fieldset>

        <fieldset>
          <legend>{i18n.t("tc.ui.execution") as TranslationKey}</legend>
          {#each EXECUTION_OPTIONS as opt (opt.mode)}
            {@const count = marketplaceExecutionCounts(items, opt.mode)}
            <label class="filter-row" class:filter-disabled={count === 0}>
              <input
                type="checkbox"
                disabled={count === 0}
                checked={toolchain.filters.execution_modes.includes(opt.mode)}
                onchange={() =>
                  toolchain.setFilters({
                    execution_modes: toggleInArray(toolchain.filters.execution_modes, opt.mode),
                  })}
              />
              <span class="filter-label">{opt.label}</span>
              <span class="filter-count">{count}</span>
            </label>
          {/each}
        </fieldset>

        <fieldset>
          <legend>{i18n.t("tc.ui.quick_filters") as TranslationKey}</legend>
          {#each QUICK_FILTERS as opt (opt.key)}
            {@const count = marketplaceBooleanCount(items, opt.key)}
            <label class="filter-row" class:filter-disabled={count === 0}>
              <input
                type="checkbox"
                disabled={count === 0}
                checked={toolchain.filters[opt.key]}
                onchange={() =>
                  toolchain.setFilters({ [opt.key]: !toolchain.filters[opt.key] })}
              />
              <span class="filter-label">{opt.label}</span>
              <span class="filter-count">{count}</span>
            </label>
          {/each}
        </fieldset>
      </div>
    </aside>

    <!-- ===== Сетка карточек ===== -->
    <div class="results">
      <p class="results-count" role="status">
        {visibleItems.length} {i18n.t("tc.ui.of") as TranslationKey} {items.length} {i18n.t("tc.ui.tools") as TranslationKey}
        {#if visibleItems.length !== items.length}
          <button type="button" class="link-btn" onclick={() => toolchain.resetFilters()}>
            {i18n.t("tc.ui.reset_filters") as TranslationKey}
          </button>
        {/if}
      </p>

      {#if toolchain.definitionsLoading}
        <LoadingState label={i18n.t("tc.market.loading_catalog") as TranslationKey} />
      {:else if toolchain.definitionsError && Object.keys(toolchain.definitions).length === 0}
        <ErrorState
          title={i18n.t("tc.market.catalog_unavailable") as TranslationKey}
          message={toolchain.definitionsError}
          retry={() => void toolchain.ensureDefinitions()}
        />
      {:else if !os && toolchain.marketplaceOsError}
        <ErrorState
          title={i18n.t("tc.market.platform_error") as TranslationKey}
          message={toolchain.marketplaceOsError}
          retry={() => void toolchain.ensureMarketplacePlatform()}
        />
      {:else if !os}
        <EmptyState
          icon="info"
          title={i18n.t("tc.market.detecting") as TranslationKey}
          description={i18n.t("tc.market.detecting_desc") as TranslationKey}
        />
      {:else if items.length === 0}
        <EmptyState
          icon="store"
          title={i18n.t("tc.market.catalog_empty") as TranslationKey}
          description={i18n.t("tc.market.catalog_empty_desc") as TranslationKey}
        />
      {:else if visibleItems.length === 0}
        <EmptyState
          compact
          icon="search"
          title={i18n.t("tc.ui.no_results") as TranslationKey}
          description={i18n.t("tc.ui.no_results_desc") as TranslationKey}
        >
          {#snippet action()}
            <Button variant="secondary" size="sm" icon="refresh" onclick={() => toolchain.resetFilters()}>
              {i18n.t("tc.ui.reset_filters") as TranslationKey}
            </Button>
          {/snippet}
        </EmptyState>
      {:else}
        <div class="grid">
          
          {#each visibleItems as item, i (item.def.id)}
            {#if i > 0 && sortWeight(visibleItems[i-1]) < 3 && sortWeight(item) >= 3}
              <div class="marketplace-boundary" style="grid-column: 1 / -1; display: flex; align-items: center; gap: 1rem; margin: 1.5rem 0 0.5rem 0;">
                <hr style="flex-grow: 1; border: none; border-top: 1px dashed var(--sp-border);" />
                <span style="font-size: 0.85rem; color: var(--sp-text-3); text-transform: uppercase; font-weight: 500; letter-spacing: 0.05em;">{i18n.t("tc.ui.installed_tools") as TranslationKey}</span>
                <hr style="flex-grow: 1; border: none; border-top: 1px dashed var(--sp-border);" />
              </div>
            {/if}
            <MarketplaceCard

              {item}
              busy={busyIds.has(item.def.id)}
              onplan={onplan}
              ondetails={ondetails}
            />
          {/each}
        </div>
      {/if}

      {#if items.length > 0 && !snapshot}
        <p class="scan-note" role="status">
          <Icon name="info" size={13} />
          {i18n.t("tc.ui.scanning_note") as TranslationKey}
        </p>
      {/if}
    </div>
  </div>
</div>

<style>
  .market {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    min-width: 0;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    flex-wrap: wrap;
  }

  .search {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex: 1 1 18rem;
    min-width: 14rem;
    padding: var(--sp-2) var(--sp-3);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    background: var(--sp-glass-bg);
    color: var(--sp-text-3);
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }

  .search:focus-within {
    border-color: var(--sp-accent-border);
    box-shadow: var(--sp-shadow-accent);
  }

  .search input {
    flex: 1 1 auto;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--sp-text-1);
    font-size: var(--sp-fs-sm);
    outline: none;
  }

  .search input::placeholder {
    color: var(--sp-text-3);
  }

  .content {
    display: grid;
    grid-template-columns: 16rem minmax(0, 1fr);
    gap: var(--sp-5);
    align-items: start;
  }

  @media (max-width: 980px) {
    .content {
      grid-template-columns: minmax(0, 1fr);
    }
    .rail {
      display: none;
    }
    .rail.rail-open {
      display: block;
    }
  }

  @media (min-width: 981px) {
    .rail-toggle-wrap {
      display: none;
    }
  }

  .rail {
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    background: var(--sp-bg-1);
    overflow: hidden;
    max-height: 70vh;
    position: sticky;
    top: 0;
  }

  .rail-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--sp-3) var(--sp-4);
    border-bottom: 1px solid var(--sp-border-faint);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .rail-body {
    padding: var(--sp-2) var(--sp-4) var(--sp-4);
    overflow-y: auto;
    max-height: calc(70vh - 3.5rem);
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .rail-note {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-2);
    margin: 0;
    padding: var(--sp-2) var(--sp-3);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border-faint);
    border-radius: var(--sp-radius-md);
  }

  fieldset {
    border: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  legend {
    padding: 0;
    margin-bottom: var(--sp-1);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--sp-text-3);
  }

  .filter-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    cursor: pointer;
    user-select: none;
  }

  .filter-row:hover {
    color: var(--sp-text-1);
  }

  .filter-disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .filter-row input {
    accent-color: var(--sp-accent-strong);
    width: 0.85rem;
    height: 0.85rem;
  }

  .filter-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .filter-count {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
    color: var(--sp-text-3);
  }

  .results {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    min-width: 0;
  }

  .results-count {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(19rem, 100%), 1fr));
    gap: var(--sp-4);
  }

  .scan-note {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    margin: 0;
    padding: var(--sp-2) var(--sp-3);
    font-size: var(--sp-fs-xs);
    color: var(--sp-cyan);
    background: var(--sp-info-soft);
    border: 1px solid var(--sp-info-border);
    border-radius: var(--sp-radius-md);
  }

  .link-btn {
    padding: 0;
    border: none;
    background: transparent;
    color: var(--sp-accent);
    font-size: inherit;
    cursor: pointer;
  }

  .link-btn:hover {
    text-decoration: underline;
  }
</style>