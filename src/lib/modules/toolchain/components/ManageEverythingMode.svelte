<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  // Режим «Управлять всем»: поиск, компактные фильтры-меню и сортировка над
  // полным каталогом. Фильтрация/сортировка — единственное, что решает
  // фронтенд; статусы и факты приходят только из снапшота бэкенда.
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
    sortCatalogTools,
  } from "../filters";
  import {
    allProvenanceKinds,
    allToolStateKinds,
    capabilityLabel,
  } from "../format";
  import type {
    CatalogSort,
    ExecutionMode,
    HealthState,
  } from "../types";

  let {
    onplan,
    onrecheck,
  }: {
    onplan: (operation: CardPlanOp, toolId: string) => void;
    onrecheck: (toolId: string) => void;
  } = $props();

  let sort = $state<CatalogSort>("status");
  let openMenu = $state<string | null>(null);
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

  // ---- Активные фильтры (лёгкие теги с очисткой) ----

  const activeTagRows = $derived.by(() => {
    const f = toolchain.filters;
    const rows: { key: string; label: string; clear: () => void }[] = [];
    for (const kind of f.states) {
      const info = allToolStateKinds().find((s) => s.kind === kind)?.info;
      if (info) {
        rows.push({
          key: `state-${kind}`,
          label: info.label,
          clear: () => toolchain.setFilters({ states: f.states.filter((x) => x !== kind) }),
        });
      }
    }
    for (const kind of f.health) {
      const opt = HEALTH_OPTIONS.find((o) => o.kind === kind);
      if (opt) {
        rows.push({
          key: `health-${kind}`,
          label: opt.label,
          clear: () => toolchain.setFilters({ health: f.health.filter((x) => x !== kind) }),
        });
      }
    }
    for (const cat of f.categories) {
      rows.push({
        key: `cat-${cat}`,
        label: cat,
        clear: () => toolchain.setFilters({ categories: f.categories.filter((x) => x !== cat) }),
      });
    }
    for (const kind of f.provenance) {
      const info = allProvenanceKinds().find((p) => p.kind === kind)?.info;
      if (info) {
        rows.push({
          key: `prov-${kind}`,
          label: info.label,
          clear: () => toolchain.setFilters({ provenance: f.provenance.filter((x) => x !== kind) }),
        });
      }
    }
    for (const flag of f.capabilities) {
      rows.push({
        key: `cap-${flag}`,
        label: capabilityLabel(flag),
        clear: () => toolchain.setFilters({ capabilities: f.capabilities.filter((x) => x !== flag) }),
      });
    }
    for (const mode of f.execution_modes) {
      const opt = EXECUTION_OPTIONS.find((o) => o.mode === mode);
      if (opt) {
        rows.push({
          key: `exec-${mode}`,
          label: opt.label,
          clear: () => toolchain.setFilters({ execution_modes: f.execution_modes.filter((x) => x !== mode) }),
        });
      }
    }
    for (const q of QUICK_FILTERS) {
      if (f[q.key]) {
        rows.push({
          key: `quick-${q.key}`,
          label: q.label,
          clear: () => toolchain.setFilters({ [q.key]: false }),
        });
      }
    }
    return rows;
  });

  const hasActiveFilters = $derived(activeTagRows.length > 0);
  const resultsHidden = $derived(
    toolchain.filters.search.length > 0 ||
      activeTagRows.length > 0 ||
      visibleTools.length !== totalTools,
  );
</script>

<div class="manage">
  <!-- ===== Единая строка поиска / фильтров / сортировки ===== -->
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

    <div class="menus">
      {#if snapshot}
        <!-- Состояния -->
        <div class="menu-wrap">
          <button
            type="button"
            class="menu-btn"
            class:menu-active={toolchain.filters.states.length > 0 || openMenu === "states"}
            onclick={() => (openMenu = openMenu === "states" ? null : "states")}
            aria-haspopup="menu"
            aria-expanded={openMenu === "states"}
          >
            <span>{i18n.t("tc.ui.status") as TranslationKey}</span>
            {#if toolchain.filters.states.length > 0}
              <span class="menu-badge">{toolchain.filters.states.length}</span>
            {/if}
            <span class="caret" aria-hidden="true"></span>
          </button>
          {#if openMenu === "states"}
            <div class="menu-panel" role="menu">
              {#each allToolStateKinds() as { kind, info } (kind)}
                {@const count = stateCounts.get(kind) ?? 0}
                {#if count > 0}
                  <label class="menu-row">
                    <input
                      type="checkbox"
                      checked={toolchain.filters.states.includes(kind)}
                      onchange={() =>
                        toolchain.setFilters({
                          states: toggleInArray(toolchain.filters.states, kind),
                        })}
                    />
                    <span class="menu-label">{info.label}</span>
                    <span class="menu-count">{count}</span>
                  </label>
                {/if}
              {/each}
            </div>
          {/if}
        </div>

        <!-- Категории -->
        <div class="menu-wrap">
          <button
            type="button"
            class="menu-btn"
            class:menu-active={toolchain.filters.categories.length > 0 || openMenu === "categories"}
            onclick={() => (openMenu = openMenu === "categories" ? null : "categories")}
            aria-haspopup="menu"
            aria-expanded={openMenu === "categories"}
          >
            <span>{i18n.t("tc.ui.categories") as TranslationKey}</span>
            {#if toolchain.filters.categories.length > 0}
              <span class="menu-badge">{toolchain.filters.categories.length}</span>
            {/if}
            <span class="caret" aria-hidden="true"></span>
          </button>
          {#if openMenu === "categories"}
            <div class="menu-panel" role="menu">
              {#each categories as cat (cat)}
                <label class="menu-row">
                  <input
                    type="checkbox"
                    checked={toolchain.filters.categories.includes(cat)}
                    onchange={() =>
                      toolchain.setFilters({
                        categories: toggleInArray(toolchain.filters.categories, cat),
                      })}
                  />
                  <span class="menu-label">{cat}</span>
                  <span class="menu-count">{categoryCounts.get(cat) ?? 0}</span>
                </label>
              {/each}
            </div>
          {/if}
        </div>

        <!-- Здоровье -->
        <div class="menu-wrap">
          <button
            type="button"
            class="menu-btn"
            class:menu-active={toolchain.filters.health.length > 0 || openMenu === "health"}
            onclick={() => (openMenu = openMenu === "health" ? null : "health")}
            aria-haspopup="menu"
            aria-expanded={openMenu === "health"}
          >
            <span>{i18n.t("tc.ui.health") as TranslationKey}</span>
            {#if toolchain.filters.health.length > 0}
              <span class="menu-badge">{toolchain.filters.health.length}</span>
            {/if}
            <span class="caret" aria-hidden="true"></span>
          </button>
          {#if openMenu === "health"}
            <div class="menu-panel" role="menu">
              {#each HEALTH_OPTIONS as opt (opt.kind)}
                {@const count = healthCounts.get(opt.kind) ?? 0}
                <label class="menu-row" class:menu-row-disabled={count === 0}>
                  <input
                    type="checkbox"
                    disabled={count === 0}
                    checked={toolchain.filters.health.includes(opt.kind)}
                    onchange={() =>
                      toolchain.setFilters({
                        health: toggleInArray(toolchain.filters.health, opt.kind),
                      })}
                  />
                  <span class="menu-label">{opt.label}</span>
                  <span class="menu-count">{count}</span>
                </label>
              {/each}
            </div>
          {/if}
        </div>

        <!-- Происхождение -->
        <div class="menu-wrap">
          <button
            type="button"
            class="menu-btn"
            class:menu-active={toolchain.filters.provenance.length > 0 || openMenu === "provenance"}
            onclick={() => (openMenu = openMenu === "provenance" ? null : "provenance")}
            aria-haspopup="menu"
            aria-expanded={openMenu === "provenance"}
          >
            <span>{i18n.t("tc.ui.origin") as TranslationKey}</span>
            {#if toolchain.filters.provenance.length > 0}
              <span class="menu-badge">{toolchain.filters.provenance.length}</span>
            {/if}
            <span class="caret" aria-hidden="true"></span>
          </button>
          {#if openMenu === "provenance"}
            <div class="menu-panel" role="menu">
              {#each allProvenanceKinds() as { kind, info } (kind)}
                {@const count = provenanceCounts.get(kind) ?? 0}
                <label class="menu-row" class:menu-row-disabled={count === 0}>
                  <input
                    type="checkbox"
                    disabled={count === 0}
                    checked={toolchain.filters.provenance.includes(kind)}
                    onchange={() =>
                      toolchain.setFilters({
                        provenance: toggleInArray(toolchain.filters.provenance, kind),
                      })}
                  />
                  <span class="menu-label">{info.label}</span>
                  <span class="menu-count">{count}</span>
                </label>
              {/each}
            </div>
          {/if}
        </div>

        <!-- Прочее: возможности, исполнение -->
        <div class="menu-wrap">
          <button
            type="button"
            class="menu-btn"
            class:menu-active={(toolchain.filters.capabilities.length > 0 || toolchain.filters.execution_modes.length > 0) || openMenu === "more"}
            onclick={() => (openMenu = openMenu === "more" ? null : "more")}
            aria-haspopup="menu"
            aria-expanded={openMenu === "more"}
          >
            <span>{i18n.t("tc.ui.more") as TranslationKey}</span>
            {#if toolchain.filters.capabilities.length > 0 || toolchain.filters.execution_modes.length > 0}
              <span class="menu-badge">{toolchain.filters.capabilities.length + toolchain.filters.execution_modes.length}</span>
            {/if}
            <span class="caret" aria-hidden="true"></span>
          </button>
          {#if openMenu === "more"}
            <div class="menu-panel" role="menu">
              <span class="menu-group">{i18n.t("tc.ui.execution") as TranslationKey}</span>
              {#each EXECUTION_OPTIONS as opt (opt.mode)}
                {@const count = executionCounts[opt.mode]}
                <label class="menu-row" class:menu-row-disabled={count === 0}>
                  <input
                    type="checkbox"
                    disabled={count === 0}
                    checked={toolchain.filters.execution_modes.includes(opt.mode)}
                    onchange={() =>
                      toolchain.setFilters({
                        execution_modes: toggleInArray(toolchain.filters.execution_modes, opt.mode),
                      })}
                  />
                  <span class="menu-label">{opt.label}</span>
                  <span class="menu-count">{count}</span>
                </label>
              {/each}
              <span class="menu-group">{i18n.t("tc.ui.capabilities") as TranslationKey}</span>
              {#each capabilityOptions as flag (flag)}
                {@const count = capabilityCounts.get(flag) ?? 0}
                <label class="menu-row" class:menu-row-disabled={count === 0}>
                  <input
                    type="checkbox"
                    disabled={count === 0}
                    checked={toolchain.filters.capabilities.includes(flag)}
                    onchange={() =>
                      toolchain.setFilters({
                        capabilities: toggleInArray(toolchain.filters.capabilities, flag),
                      })}
                  />
                  <span class="menu-label">{capabilityLabel(flag)}</span>
                  <span class="menu-count">{count}</span>
                </label>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <label class="sort">
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
  </div>

  {#if openMenu}
    <button
      type="button"
      class="menu-backdrop"
      aria-label={i18n.t("tc.ui.clear") as TranslationKey}
      onclick={() => (openMenu = null)}
    ></button>
  {/if}

  <!-- ===== Быстрые фильтры (лёгкие чипы) ===== -->
  <div class="chips">
    {#each QUICK_FILTERS as opt (opt.key)}
      {@const count = quickFilterCount(opt.key)}
      {@const active = toolchain.filters[opt.key]}
      <button
        type="button"
        class="chip"
        class:chip-active={active}
        disabled={count === 0 && !active}
        onclick={() => toolchain.setFilters({ [opt.key]: !active })}
      >
        {opt.label}
        {#if count > 0}
          <span class="chip-count">{count}</span>
        {/if}
      </button>
    {/each}
  </div>

  <!-- ===== Активные фильтры: теги с очисткой ===== -->
  {#if hasActiveFilters}
    <div class="tags">
      {#each activeTagRows as tag (tag.key)}
        <span class="tag">
          {tag.label}
          <button
            type="button"
            class="tag-clear"
            aria-label={i18n.t("tc.ui.clear") as TranslationKey}
            onclick={tag.clear}
          >
            <Icon name="x" size={11} />
          </button>
        </span>
      {/each}
      <button type="button" class="link-btn" onclick={() => toolchain.resetFilters()}>
        {i18n.t("tc.ui.reset_all") as TranslationKey}
      </button>
    </div>
  {/if}

  <div class="results">
    <p class="results-count" role="status">
      {visibleTools.length} {i18n.t("tc.ui.of") as TranslationKey} {totalTools} {i18n.t("tc.ui.tools") as TranslationKey}
      {#if resultsHidden}
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
      <div class="list">
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

<style>
  .manage {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    min-width: 0;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }

  .search {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex: 1 1 16rem;
    min-width: 12rem;
    padding: var(--sp-1) var(--sp-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    background: var(--sp-bg-1);
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

  .menus {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-wrap: wrap;
  }

  .menu-wrap {
    position: relative;
  }

  .menu-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: var(--sp-1) var(--sp-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-sm);
    background: var(--sp-bg-1);
    color: var(--sp-text-2);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-medium);
    cursor: pointer;
    white-space: nowrap;
    user-select: none;
    transition: border-color 0.15s ease, background-color 0.15s ease, color 0.15s ease;
  }

  .menu-btn:hover,
  .menu-btn.menu-active {
    border-color: var(--sp-border-strong);
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }

  .menu-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1rem;
    height: 1rem;
    padding: 0 var(--sp-1);
    border-radius: var(--sp-radius-full);
    background: var(--sp-accent-soft);
    color: var(--sp-accent);
    font-size: 0.625rem;
    font-weight: var(--sp-fw-semibold);
  }

  .caret {
    width: 0;
    height: 0;
    margin-left: var(--sp-1);
    border-left: 3px solid transparent;
    border-right: 3px solid transparent;
    border-top: 4px solid currentColor;
    opacity: 0.7;
  }

  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 90;
    border: none;
    padding: 0;
    background: transparent;
    cursor: default;
  }

  .menu-panel {
    position: absolute;
    top: calc(100% + var(--sp-1));
    left: 0;
    z-index: 100;
    min-width: 15rem;
    max-width: 20rem;
    max-height: min(24rem, 60vh);
    overflow-y: auto;
    padding: var(--sp-2);
    background: var(--sp-glass-strong);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-md);
    box-shadow: var(--sp-shadow-2);
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  .menu-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-1);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    cursor: pointer;
    user-select: none;
    border-radius: var(--sp-radius-xs);
  }

  .menu-row:hover {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }

  .menu-row input {
    accent-color: var(--sp-accent-strong);
    width: 0.85rem;
    height: 0.85rem;
    flex: 0 0 auto;
  }

  .menu-row-disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .menu-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1 1 auto;
  }

  .menu-count {
    flex: 0 0 auto;
    font-variant-numeric: tabular-nums;
    color: var(--sp-text-3);
  }

  .menu-group {
    padding: var(--sp-1) var(--sp-1) 0;
    font-size: 0.625rem;
    font-weight: var(--sp-fw-semibold);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--sp-text-3);
  }

  .sort select {
    padding: var(--sp-1) var(--sp-2);
    border-radius: var(--sp-radius-sm);
    border: 1px solid var(--sp-border);
    background: var(--sp-bg-1);
    color: var(--sp-text-1);
    font-size: var(--sp-fs-xs);
    max-width: 10rem;
  }

  .chips {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-wrap: wrap;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: 0 var(--sp-2);
    height: 1.5rem;
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-full);
    background: transparent;
    color: var(--sp-text-2);
    font-size: var(--sp-fs-xs);
    cursor: pointer;
    white-space: nowrap;
    user-select: none;
    transition: border-color 0.15s ease, background-color 0.15s ease, color 0.15s ease;
  }

  .chip:hover:not(:disabled) {
    border-color: var(--sp-border-strong);
    color: var(--sp-text-1);
  }

  .chip-active {
    background: var(--sp-accent-soft);
    border-color: var(--sp-accent-border);
    color: var(--sp-accent);
  }

  .chip:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .chip-count {
    font-variant-numeric: tabular-nums;
    opacity: 0.8;
  }

  .tags {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-wrap: wrap;
  }

  .tag {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: 0 var(--sp-1) 0 var(--sp-2);
    height: 1.5rem;
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-full);
    background: var(--sp-bg-1);
    color: var(--sp-text-2);
    font-size: var(--sp-fs-xs);
    white-space: nowrap;
  }

  .tag-clear {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1rem;
    height: 1rem;
    padding: 0;
    border: none;
    border-radius: var(--sp-radius-full);
    background: transparent;
    color: var(--sp-text-3);
    cursor: pointer;
  }

  .tag-clear:hover {
    background: var(--sp-bg-3);
    color: var(--sp-text-1);
  }

  .results {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    min-width: 0;
  }

  .results-count {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    min-width: 0;
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