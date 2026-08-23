<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  // Режим «Управлять всем»: поиск, рейло фильтров и сортировка над полным
  // каталогом. Фильтрация/сортировка — единственное, что решает фронтенд;
  // статусы и факты приходят только из снапшота бэкенда.
  import Button from "$lib/components/ui/Button.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import ToolCard from "./ToolCard.svelte";
  import { toolchain } from "../state.svelte";
  import type { CardPlanOp } from "./ToolCard.svelte";
  import {
    applyCatalogFilters,
    availableCategories,
    defaultCatalogFilters,
    sortCatalogTools,
  } from "../filters";
  import {
    allProvenanceKinds,
    allToolStateKinds,
    capabilityLabel,
  } from "../format";
  import type { CatalogSort, ExecutionMode, HealthState } from "../types";

  let {
    onplan,
    onrecheck,
  }: {
    onplan: (operation: CardPlanOp, toolId: string) => void;
    onrecheck: (toolId: string) => void;
  } = $props();

  let sort = $state<CatalogSort>("status");
  let railOpen = $state(false);
  let recheckingIds = $state<Set<string>>(new Set());

  const snapshot = $derived(toolchain.liveSnapshot);
  const categories = $derived(availableCategories(snapshot));

  const categoryCounts = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const t of snapshot?.tools ?? []) {
      counts.set(t.category, (counts.get(t.category) ?? 0) + 1);
    }
    return counts;
  });

  const stateCounts = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const t of snapshot?.tools ?? []) {
      const kind = t.state.kind;
      counts.set(kind, (counts.get(kind) ?? 0) + 1);
    }
    return counts;
  });

  /** Счётчики для каждой группы фильтров — только по текущему набору. */
  const healthCounts = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const t of snapshot?.tools ?? []) {
      const kind = t.health?.state.kind ?? "not_checked";
      counts.set(kind, (counts.get(kind) ?? 0) + 1);
    }
    return counts;
  });

  const provenanceCounts = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const t of snapshot?.tools ?? []) {
      const kind = t.provenance.kind;
      counts.set(kind, (counts.get(kind) ?? 0) + 1);
    }
    return counts;
  });

  const capabilityCounts = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const t of snapshot?.tools ?? []) {
      for (const flag of Object.keys(t.capabilities) as (keyof typeof t.capabilities)[]) {
        if (t.capabilities[flag]) counts.set(flag, (counts.get(flag) ?? 0) + 1);
      }
    }
    return counts;
  });

  const executionCounts = $derived.by(() => {
    let host = 0;
    let docker = 0;
    for (const t of snapshot?.tools ?? []) {
      if (t.applicability.kind === "docker_default" || t.provenance.kind === "docker") {
        docker += 1;
      } else {
        host += 1;
      }
    }
    return { host, docker };
  });

  const quickCounts = $derived.by(() => {
    let updateOnly = 0;
    let adminOnly = 0;
    let manualOnly = 0;
    let installable = 0;
    let dockerAlt = 0;
    for (const t of snapshot?.tools ?? []) {
      if (t.state.kind === "update_available") updateOnly += 1;
      if (t.applicability.kind === "manual_only") manualOnly += 1;
      if (t.capabilities.installable) installable += 1;
      if (t.capabilities.docker_alternative_available) dockerAlt += 1;
      const def = toolchain.definitionFor(t.tool_id);
      if (def?.needs_admin === true) adminOnly += 1;
    }
    return { updateOnly, adminOnly, manualOnly, installable, dockerAlt };
  });

  const visibleTools = $derived(
    sortCatalogTools(
      applyCatalogFilters(snapshot, toolchain.filters, {
        definitions: toolchain.definitions,
      }),
      sort,
    ),
  );

  const totalTools = $derived(snapshot?.tools.length ?? 0);

  function toggleInArray<T>(arr: T[], value: T): T[] {
    return arr.includes(value) ? arr.filter((x) => x !== value) : [...arr, value];
  }

  async function handleRecheck(toolId: string): Promise<void> {
    if (recheckingIds.has(toolId)) return;
    recheckingIds = new Set([...recheckingIds, toolId]);
    try {
      await toolchain.runHealthChecks([toolId]);
    } finally {
      const next = new Set(recheckingIds);
      next.delete(toolId);
      recheckingIds = next;
    }
  }

  const HEALTH_OPTIONS: { kind: HealthState["kind"]; label: TranslationKey }[] = [
    { kind: "healthy", label: i18n.t("tc.health.healthy") as TranslationKey },
    { kind: "degraded", label: i18n.t("tc.health.degraded") as TranslationKey },
    { kind: "unhealthy", label: i18n.t("tc.health.unhealthy") as TranslationKey },
    { kind: "not_checked", label: i18n.t("tc.health.not_checked") as TranslationKey },
  ];

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

  function quickFilterCount(
    key: (typeof QUICK_FILTERS)[number]["key"],
  ): number {
    switch (key) {
      case "update_only":
        return quickCounts.updateOnly;
      case "admin_only":
        return quickCounts.adminOnly;
      case "manual_only":
        return quickCounts.manualOnly;
      case "installable":
        return quickCounts.installable;
      case "has_docker_alternative":
        return quickCounts.dockerAlt;
    }
  }

  const EXECUTION_OPTIONS: { mode: ExecutionMode; label: TranslationKey }[] = [
    { mode: "host", label: i18n.t("tc.ui.exec.host") as TranslationKey },
    { mode: "docker", label: i18n.t("tc.ui.exec.docker") as TranslationKey },
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

  const SORT_OPTIONS: { value: CatalogSort; label: TranslationKey }[] = [
    { value: "status", label: i18n.t("tc.ui.sort.problematic") as TranslationKey },
    { value: "name_asc", label: i18n.t("tc.ui.sort.name_asc") as TranslationKey },
    { value: "name_desc", label: i18n.t("tc.ui.sort.name_desc") as TranslationKey },
    { value: "category", label: i18n.t("tc.ui.sort.category") as TranslationKey },
    { value: "catalog", label: i18n.t("tc.ui.sort.catalog") as TranslationKey },
  ];
</script>

<div class="manage">
  <!-- ===== Панель поиска и сортировки ===== -->
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

    <label class="sort">
      <span class="sort-label">{i18n.t("tc.ui.sorting") as TranslationKey}</span>
      <select
        value={sort}
        onchange={(e) => (sort = e.currentTarget.value as CatalogSort)}
        aria-label={i18n.t("tc.ui.sort_aria") as TranslationKey}
      >
        {#each SORT_OPTIONS as opt (opt.value)}
          <option value={opt.value}>{opt.label}</option>
        {/each}
      </select>
    </label>

    <div class="rail-toggle-wrap">
      <Button variant="ghost" size="sm" icon={railOpen ? "x" : "layers"} onclick={() => (railOpen = !railOpen)}>
        {i18n.t("tc.ui.filters") as TranslationKey}
      </Button>
    </div>
  </div>

  <div class="content">
    <!-- ===== Рейло фильтров ===== -->
    <aside class="rail" class:rail-open={railOpen} aria-label={i18n.t("tc.ui.catalog_filters") as TranslationKey}>
      <div class="rail-head">
        <span>{i18n.t("tc.ui.filters") as TranslationKey}</span>
        <Button variant="ghost" size="sm" onclick={() => toolchain.resetFilters()}>
          {i18n.t("tc.ui.reset_all") as TranslationKey}
        </Button>
      </div>

      <div class="rail-body">
        <!-- Состояния -->
        <fieldset>
          <legend>{i18n.t("tc.ui.status") as TranslationKey}</legend>
          {#each allToolStateKinds() as { kind, info } (kind)}
            {@const count = stateCounts.get(kind) ?? 0}
            {#if count > 0}
              <label class="filter-row">
                <input
                  type="checkbox"
                  checked={toolchain.filters.states.includes(kind)}
                  onchange={() =>
                    toolchain.setFilters({
                      states: toggleInArray(toolchain.filters.states, kind),
                    })}
                />
                <span class="filter-label">{info.label}</span>
                <span class="filter-count">{count}</span>
              </label>
            {/if}
          {/each}
        </fieldset>

        <!-- Здоровье -->
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

        <!-- Категории -->
        <fieldset>
          <legend>{i18n.t("tc.ui.categories") as TranslationKey}</legend>
          {#each categories as cat (cat)}
            <label class="filter-row">
              <input
                type="checkbox"
                checked={toolchain.filters.categories.includes(cat)}
                onchange={() =>
                  toolchain.setFilters({
                    categories: toggleInArray(toolchain.filters.categories, cat),
                  })}
              />
              <span class="filter-label">{cat}</span>
              <span class="filter-count">{categoryCounts.get(cat) ?? 0}</span>
            </label>
          {/each}
        </fieldset>

        <!-- Происхождение -->
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

        <!-- Возможности платформы -->
        <fieldset>
          <legend>{i18n.t("tc.ui.capabilities") as TranslationKey}</legend>
          {#each capabilityOptions as flag (flag)}
            {@const count = capabilityCounts.get(flag) ?? 0}
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

        <!-- Режим исполнения -->
        <fieldset>
          <legend>{i18n.t("tc.ui.execution") as TranslationKey}</legend>
          {#each EXECUTION_OPTIONS as opt (opt.mode)}
            {@const count = executionCounts[opt.mode]}
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

        <!-- Быстрые фильтры -->
        <fieldset>
          <legend>{i18n.t("tc.ui.quick_filters") as TranslationKey}</legend>
          {#each QUICK_FILTERS as opt (opt.key)}
            {@const count = quickFilterCount(opt.key)}
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
        {visibleTools.length} {i18n.t("tc.ui.of") as TranslationKey} {totalTools} {i18n.t("tc.ui.tools") as TranslationKey}
        {#if visibleTools.length !== totalTools}
          <button type="button" class="link-btn" onclick={() => toolchain.resetFilters()}>
            {i18n.t("tc.ui.reset_filters") as TranslationKey}
          </button>
        {/if}
      </p>

      {#if toolchain.snapshotLoading && !snapshot}
        <LoadingState label={i18n.t("tc.ui.loading_env") as TranslationKey} />
      {:else if toolchain.snapshotError && !snapshot}
        <ErrorState
          title={i18n.t("tc.ui.load_error") as TranslationKey}
          message={toolchain.snapshotError}
          retry={() => void toolchain.refreshSnapshot()}
        />
      {:else if !snapshot || snapshot.tools.length === 0}
        <EmptyState
          icon="wrench"
          title={i18n.t("tc.ui.no_data_title") as TranslationKey}
          description={i18n.t("tc.ui.no_data_desc") as TranslationKey}
        >
          {#snippet action()}
            <Button variant="primary" icon="refresh" loading={toolchain.snapshotLoading} onclick={() => void toolchain.ensureScanRunning()}>
              {i18n.t("tc.ui.run_scan") as TranslationKey}
            </Button>
          {/snippet}
        </EmptyState>
      {:else if visibleTools.length === 0}
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
          {#each visibleTools as tool (tool.tool_id)}
            <ToolCard
              {tool}
              def={toolchain.definitionFor(tool.tool_id)}
              busy={recheckingIds.has(tool.tool_id)}
              ondetails={(id) => toolchain.selectTool(id)}
              onplan={onplan}
              onrecheck={handleRecheck}
              onuninstall={(id) => toolchain.uninstallTool(id)}
            />
          {/each}
        </div>
      {/if}

      {#if snapshot && snapshot.tools.some((t) => t.state.kind === "scan_pending")}
        <p class="scan-note" role="status">
          <Icon name="clock" size={13} />
          {i18n.t("tc.ui.scanning_note") as TranslationKey}
        </p>
      {/if}
    </div>
  </div>
</div>

<style>
  .manage {
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

  .sort {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .sort-label {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    white-space: nowrap;
  }

  .sort select {
    padding: var(--sp-1) var(--sp-2);
    border-radius: var(--sp-radius-sm);
    border: 1px solid var(--sp-border);
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
    font-size: var(--sp-fs-sm);
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

  .filter-row input {
    accent-color: var(--sp-accent-strong);
    width: 0.85rem;
    height: 0.85rem;
  }

  .filter-disabled {
    opacity: 0.45;
    cursor: not-allowed;
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
    background: rgba(34, 211, 238, 0.07);
    border: 1px solid rgba(34, 211, 238, 0.25);
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
