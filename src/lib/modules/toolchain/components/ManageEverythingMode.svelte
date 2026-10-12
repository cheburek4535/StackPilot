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
  import ToolchainSection from "./ToolchainSection.svelte";
  import {
    TOOLCHAIN_SECTIONS,
    getToolSectionId,
    getToolPopularityRank,
    type ToolchainSectionId,
    type ToolchainSectionMeta,
  } from "../sections";

  let {
    onplan,
    onrecheck,
  }: {
    onplan: (operation: CardPlanOp, toolId: string) => void;
    onrecheck: (toolId: string) => void;
  } = $props();

  let sort = $state<CatalogSort>("status");
  let filterMenuOpen = $state(false);
  let recheckingIds = $state<Set<string>>(new Set());
  let selectedSectionId = $state<ToolchainSectionId | "updates" | "all">("all");

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

  const STATE_SEVERITY_ORDER: Record<string, number> = {
    update_available: 0,
    path_broken: 1,
    installed_unhealthy: 2,
    scan_failed: 3,
    missing: 4,
    installed_health_unknown: 5,
    scan_pending: 6,
    manual_install: 7,
    docker_managed: 8,
    unsupported_platform: 9,
    built_in_system: 10,
    installed_healthy: 11,
  };

  type ManageSectionGroup = {
    meta: ToolchainSectionMeta;
    items: typeof visibleTools;
    installedCount: number;
    updateCount: number;
  };

  const updateTools = $derived(
    visibleTools.filter((t) => t.state.kind === "update_available"),
  );

  const sectionGroups = $derived.by(() => {
    const map = new Map<ToolchainSectionId, typeof visibleTools>();
    for (const sec of TOOLCHAIN_SECTIONS) {
      map.set(sec.id, []);
    }

    for (const tool of visibleTools) {
      const secId = getToolSectionId(tool.tool_id, tool.category);
      const list = map.get(secId);
      if (list) {
        list.push(tool);
      } else {
        map.get("tooling")?.push(tool);
      }
    }

    const groups: ManageSectionGroup[] = [];
    for (const sec of TOOLCHAIN_SECTIONS) {
      const rawItems = map.get(sec.id) ?? [];
      if (rawItems.length === 0) continue;

      const sorted = [...rawItems].sort((a, b) => {
        if (sort === "name_asc") return a.display.localeCompare(b.display);
        if (sort === "name_desc") return b.display.localeCompare(a.display);

        // Инструменты с обновлением всегда идут первыми в секции
        const aUpdate = a.state.kind === "update_available" ? 1 : 0;
        const bUpdate = b.state.kind === "update_available" ? 1 : 0;
        if (aUpdate !== bUpdate) return bUpdate - aUpdate;

        if (sort === "status") {
          const sa = STATE_SEVERITY_ORDER[a.state.kind] ?? 99;
          const sb = STATE_SEVERITY_ORDER[b.state.kind] ?? 99;
          if (sa !== sb) return sa - sb;
        }

        // По популярности
        const ra = getToolPopularityRank(a.tool_id);
        const rb = getToolPopularityRank(b.tool_id);
        if (ra !== rb) return ra - rb;

        return a.display.localeCompare(b.display);
      });

      const installedCount = sorted.filter(
        (t) =>
          t.state.kind !== "missing" &&
          t.state.kind !== "scan_pending" &&
          t.state.kind !== "scan_failed" &&
          t.state.kind !== "unsupported_platform",
      ).length;

      const updateCount = sorted.filter((t) => t.state.kind === "update_available").length;

      groups.push({
        meta: sec,
        items: sorted,
        installedCount,
        updateCount,
      });
    }

    return groups;
  });

  const displayedGroups = $derived(
    selectedSectionId === "all" || selectedSectionId === "updates"
      ? sectionGroups
      : sectionGroups.filter((g) => g.meta.id === selectedSectionId),
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
      | "admin_only"
      | "manual_only"
      | "installable"
      | "has_docker_alternative";
    label: TranslationKey;
  }[] = [
    { key: "admin_only", label: i18n.t("tc.ui.filter.admin_only") as TranslationKey },
    { key: "manual_only", label: i18n.t("tc.ui.filter.manual_only") as TranslationKey },
    { key: "installable", label: i18n.t("tc.ui.filter.installable") as TranslationKey },
    { key: "has_docker_alternative", label: i18n.t("tc.ui.filter.docker_alt") as TranslationKey },
  ];

  function quickFilterCount(
    key: (typeof QUICK_FILTERS)[number]["key"],
  ): number {
    switch (key) {
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

  const activeFilterCount = $derived(activeTagRows.length);
  const hasActiveFilters = $derived(activeTagRows.length > 0);
  const resultsHidden = $derived(
    toolchain.filters.search.length > 0 ||
      activeTagRows.length > 0 ||
      visibleTools.length !== totalTools,
  );
</script>

<div class="manage">
  <!-- ===== Верхняя строка: Поиск + Фильтры + Сортировка ===== -->
  <div class="toolbar">
    <div class="search">
      <Icon name="search" size={15} />
      <input
        type="search"
        placeholder={i18n.t("tc.ui.search_placeholder") as TranslationKey}
        value={toolchain.filters.search}
        oninput={(e) => toolchain.setFilters({ search: e.currentTarget.value })}
        aria-label={i18n.t("tc.ui.search_aria") as TranslationKey}
      />
      {#if toolchain.filters.search}
        <button
          type="button"
          class="search-clear-btn"
          onclick={() => toolchain.setFilters({ search: "" })}
          aria-label={i18n.t("tc.ui.clear_search") as TranslationKey}
        >
          <Icon name="x" size={13} />
        </button>
      {/if}
    </div>

    <!-- Единая кнопка фильтров с выпадающей панелью -->
    <div class="filter-wrap">
      <button
        type="button"
        class="filter-btn"
        class:filter-btn-active={hasActiveFilters || filterMenuOpen}
        onclick={() => (filterMenuOpen = !filterMenuOpen)}
        aria-haspopup="dialog"
        aria-expanded={filterMenuOpen}
      >
        <Icon name="sliders" size={14} />
        <span>{i18n.t("tc.ui.filters") as TranslationKey}</span>
        {#if activeFilterCount > 0}
          <span class="filter-badge">{activeFilterCount}</span>
        {/if}
        <span class="caret" aria-hidden="true"></span>
      </button>

      {#if filterMenuOpen}
        <div class="filter-popover" role="dialog" aria-label={i18n.t("tc.ui.catalog_filters") as TranslationKey}>
          <div class="filter-popover-header">
            <span class="filter-popover-title">{i18n.t("tc.ui.catalog_filters") as TranslationKey}</span>
            {#if hasActiveFilters}
              <button
                type="button"
                class="link-btn"
                onclick={() => toolchain.resetFilters()}
              >
                {i18n.t("tc.ui.reset_all") as TranslationKey}
              </button>
            {/if}
          </div>

          <div class="filter-popover-body">
            <!-- Состояние -->
            <div class="filter-section">
              <span class="filter-section-title">{i18n.t("tc.ui.status") as TranslationKey}</span>
              <div class="filter-options">
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
              </div>
            </div>

            <!-- Свойства инструментов -->
            <div class="filter-section">
              <span class="filter-section-title">{i18n.t("tc.ui.quick_filters") as TranslationKey}</span>
              <div class="filter-options">
                {#each QUICK_FILTERS as opt (opt.key)}
                  {@const count = quickFilterCount(opt.key)}
                  {#if count > 0}
                    <label class="filter-row">
                      <input
                        type="checkbox"
                        checked={toolchain.filters[opt.key]}
                        onchange={() =>
                          toolchain.setFilters({ [opt.key]: !toolchain.filters[opt.key] })}
                      />
                      <span class="filter-label">{opt.label}</span>
                      <span class="filter-count">{count}</span>
                    </label>
                  {/if}
                {/each}
              </div>
            </div>

            <!-- Здоровье -->
            <div class="filter-section">
              <span class="filter-section-title">{i18n.t("tc.ui.health") as TranslationKey}</span>
              <div class="filter-options">
                {#each HEALTH_OPTIONS as opt (opt.kind)}
                  {@const count = healthCounts.get(opt.kind) ?? 0}
                  {#if count > 0}
                    <label class="filter-row">
                      <input
                        type="checkbox"
                        checked={toolchain.filters.health.includes(opt.kind)}
                        onchange={() =>
                          toolchain.setFilters({
                            health: toggleInArray(toolchain.filters.health, opt.kind),
                          })}
                      />
                      <span class="filter-label">{opt.label}</span>
                      <span class="filter-count">{count}</span>
                    </label>
                  {/if}
                {/each}
              </div>
            </div>

            <!-- Происхождение -->
            <div class="filter-section">
              <span class="filter-section-title">{i18n.t("tc.ui.origin") as TranslationKey}</span>
              <div class="filter-options">
                {#each allProvenanceKinds() as { kind, info } (kind)}
                  {@const count = provenanceCounts.get(kind) ?? 0}
                  {#if count > 0}
                    <label class="filter-row">
                      <input
                        type="checkbox"
                        checked={toolchain.filters.provenance.includes(kind)}
                        onchange={() =>
                          toolchain.setFilters({
                            provenance: toggleInArray(toolchain.filters.provenance, kind),
                          })}
                      />
                      <span class="filter-label">{info.label}</span>
                      <span class="filter-count">{count}</span>
                    </label>
                  {/if}
                {/each}
              </div>
            </div>

            <!-- Исполнение -->
            <div class="filter-section">
              <span class="filter-section-title">{i18n.t("tc.ui.execution") as TranslationKey}</span>
              <div class="filter-options">
                {#each EXECUTION_OPTIONS as opt (opt.mode)}
                  {@const count = executionCounts[opt.mode]}
                  {#if count > 0}
                    <label class="filter-row">
                      <input
                        type="checkbox"
                        checked={toolchain.filters.execution_modes.includes(opt.mode)}
                        onchange={() =>
                          toolchain.setFilters({
                            execution_modes: toggleInArray(toolchain.filters.execution_modes, opt.mode),
                          })}
                      />
                      <span class="filter-label">{opt.label}</span>
                      <span class="filter-count">{count}</span>
                    </label>
                  {/if}
                {/each}
              </div>
            </div>
          </div>
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

  {#if filterMenuOpen}
    <button
      type="button"
      class="menu-backdrop"
      aria-label={i18n.t("tc.ui.clear") as TranslationKey}
      onclick={() => (filterMenuOpen = false)}
    ></button>
  {/if}

  <!-- ===== Категории инструментов (Секционные вкладки) + Результаты в одной строке ===== -->
  <div class="categories-bar">
    {#if sectionGroups.length > 1 || updateTools.length > 0}
      <div class="section-tabs" role="tablist" aria-label="Секции инструментов">
        <button
          type="button"
          role="tab"
          class="section-tab"
          class:section-tab-active={selectedSectionId === "all"}
          onclick={() => (selectedSectionId = "all")}
        >
          <span>{i18n.t("tc.section.all") as TranslationKey}</span>
          <span class="section-tab-count">{visibleTools.length}</span>
        </button>
        {#if updateTools.length > 0}
          <button
            type="button"
            role="tab"
            class="section-tab section-tab-update"
            class:section-tab-active={selectedSectionId === "updates"}
            onclick={() => (selectedSectionId = "updates")}
          >
            <Icon name="refresh" size={13} />
            <span>{i18n.t("tc.section.updates") as TranslationKey}</span>
            <span class="section-tab-count update-badge">{updateTools.length}</span>
          </button>
        {/if}
        {#each sectionGroups as group (group.meta.id)}
          <button
            type="button"
            role="tab"
            class="section-tab"
            class:section-tab-active={selectedSectionId === group.meta.id}
            onclick={() => (selectedSectionId = group.meta.id)}
          >
            <Icon name={group.meta.icon} size={13} />
            <span>{i18n.t(group.meta.titleKey as TranslationKey) || group.meta.defaultTitle}</span>
            <span class="section-tab-count">{group.items.length}</span>
          </button>
        {/each}
      </div>
    {/if}

    <div class="results-meta">
      {#if resultsHidden}
        <span class="results-text">{visibleTools.length} {i18n.t("tc.ui.of") as TranslationKey} {totalTools}</span>
        <button type="button" class="link-btn" onclick={() => toolchain.resetFilters()}>
          {i18n.t("tc.ui.reset_filters") as TranslationKey}
        </button>
      {/if}
    </div>
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
      <!-- Если выбрана вкладка "Обновления" или "Все" и есть инструменты с обновлениями, показываем их ПЕРВЫМИ! -->
      {#if updateTools.length > 0 && (selectedSectionId === "all" || selectedSectionId === "updates") && !toolchain.filters.update_only}
        <ToolchainSection
          id="tc-manage-updates"
          title={i18n.t("tc.section.updates") as TranslationKey}
          description={i18n.t("tc.section.updates_desc") as TranslationKey}
          icon="refresh"
          count={updateTools.length}
          badgeText={`${updateTools.length} ${i18n.t("tc.state.update") as TranslationKey}`}
          badgeTone="amber"
        >
          <div class="list">
            {#each updateTools as tool (tool.tool_id)}
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
        </ToolchainSection>
      {/if}

      <!-- Секции инструментов (если не выбран фильтр только обновлений) -->
      {#if selectedSectionId !== "updates"}
        {#each displayedGroups as group (group.meta.id)}
          <ToolchainSection
            id={"tc-manage-" + group.meta.id}
            title={i18n.t(group.meta.titleKey as TranslationKey) || group.meta.defaultTitle}
            description={i18n.t(group.meta.descKey as TranslationKey) || group.meta.defaultDesc}
            icon={group.meta.icon}
            count={group.items.length}
            badgeText={group.installedCount > 0 ? `${group.installedCount}/${group.items.length} ${i18n.t("tc.state.installed") as TranslationKey}` : `${group.items.length} ${i18n.t("tc.ui.tools") as TranslationKey}`}
            badgeTone={group.installedCount === group.items.length ? "lime" : group.installedCount > 0 ? "cyan" : "neutral"}
          >
            <div class="list">
              {#each group.items as tool (tool.tool_id)}
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
          </ToolchainSection>
        {/each}
      {/if}
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

  .search-clear-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    border: none;
    border-radius: var(--sp-radius-full);
    background: transparent;
    color: var(--sp-text-3);
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease;
  }

  .search-clear-btn:hover {
    background: var(--sp-bg-3);
    color: var(--sp-text-1);
  }

  .filter-wrap {
    position: relative;
  }

  .filter-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-3);
    height: 2.125rem;
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    background: var(--sp-bg-1);
    color: var(--sp-text-2);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-medium);
    cursor: pointer;
    user-select: none;
    transition: border-color 0.15s ease, background-color 0.15s ease, color 0.15s ease;
  }

  .filter-btn:hover,
  .filter-btn.filter-btn-active {
    border-color: var(--sp-accent-border, var(--sp-border-strong));
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }

  .filter-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.1rem;
    height: 1.1rem;
    padding: 0 0.3rem;
    border-radius: var(--sp-radius-full);
    background: var(--sp-accent);
    color: #fff;
    font-size: 0.65rem;
    font-weight: var(--sp-fw-bold);
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

  .filter-popover {
    position: absolute;
    top: calc(100% + var(--sp-2));
    right: 0;
    z-index: 100;
    width: 26rem;
    max-width: min(92vw, 32rem);
    max-height: min(28rem, 70vh);
    overflow-y: auto;
    padding: var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-lg);
    box-shadow: var(--sp-shadow-3, 0 8px 24px rgba(0, 0, 0, 0.4));
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .filter-popover-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-bottom: var(--sp-2);
    border-bottom: 1px solid var(--sp-border-faint);
  }

  .filter-popover-title {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .filter-popover-body {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .filter-section {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1-5, var(--sp-1));
  }

  .filter-section-title {
    font-size: 0.6875rem;
    font-weight: var(--sp-fw-semibold);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--sp-text-3);
  }

  .filter-options {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(11rem, 1fr));
    gap: var(--sp-1);
  }

  .filter-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-2);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    cursor: pointer;
    user-select: none;
    border-radius: var(--sp-radius-sm);
    transition: background 0.1s ease, color 0.1s ease;
  }

  .filter-row:hover {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }

  .filter-row input {
    accent-color: var(--sp-accent);
    width: 0.9rem;
    height: 0.9rem;
    flex: 0 0 auto;
  }

  .filter-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1 1 auto;
  }

  .filter-count {
    flex: 0 0 auto;
    font-variant-numeric: tabular-nums;
    font-size: 0.7rem;
    color: var(--sp-text-3);
  }

  .sort select {
    padding: var(--sp-1) var(--sp-2);
    height: 2.125rem;
    border-radius: var(--sp-radius-md);
    border: 1px solid var(--sp-border);
    background: var(--sp-bg-1);
    color: var(--sp-text-1);
    font-size: var(--sp-fs-xs);
    max-width: 11rem;
    cursor: pointer;
  }

  .categories-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    flex-wrap: wrap;
    padding-top: var(--sp-1);
  }

  .results-meta {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    margin-left: auto;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    white-space: nowrap;
  }

  .results-text {
    font-variant-numeric: tabular-nums;
  }

  .tags {
    display: flex;
    align-items: center;
    gap: var(--sp-1-5, var(--sp-1));
    flex-wrap: wrap;
    padding-top: var(--sp-1);
  }

  .tag {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: 0 var(--sp-1) 0 var(--sp-2);
    height: 1.5rem;
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-full);
    background: var(--sp-bg-2);
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
    gap: var(--sp-3);
    min-width: 0;
    margin-top: var(--sp-1);
  }

  .section-tabs {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    overflow-x: auto;
    padding: 2px 0 var(--sp-2) 0;
    scrollbar-width: thin;
    -webkit-overflow-scrolling: touch;
  }

  .section-tab {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-3);
    border-radius: var(--sp-radius-full, 999px);
    border: 1px solid var(--sp-border);
    background: var(--sp-glass-bg);
    color: var(--sp-text-2);
    font-size: var(--sp-fs-xs);
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: border-color 0.15s ease, background 0.15s ease, color 0.15s ease;
  }

  .section-tab:hover {
    background: var(--sp-surface-2);
    color: var(--sp-text-1);
    border-color: var(--sp-border-strong);
  }

  .section-tab-active {
    background: var(--sp-accent, #6366f1);
    color: #fff;
    border-color: var(--sp-accent, #6366f1);
    box-shadow: 0 2px 8px rgba(99, 102, 241, 0.25);
  }

  .section-tab-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.2rem;
    height: 1.2rem;
    padding: 0 4px;
    border-radius: 999px;
    font-size: 0.65rem;
    font-weight: 600;
    background: var(--sp-surface-3, rgba(255, 255, 255, 0.08));
    color: var(--sp-text-3);
  }

  .section-tab-active .section-tab-count {
    background: rgba(255, 255, 255, 0.25);
    color: #fff;
  }

  .section-tab-update {
    border-color: rgba(245, 158, 11, 0.4);
    color: #f59e0b;
  }

  .section-tab-count.update-badge {
    background: rgba(245, 158, 11, 0.2);
    color: #f59e0b;
  }

  .section-tab-update.section-tab-active {
    background: #f59e0b;
    color: #000;
    border-color: #f59e0b;
    box-shadow: 0 2px 8px rgba(245, 158, 11, 0.3);
  }

  .section-tab-update.section-tab-active .section-tab-count {
    background: rgba(0, 0, 0, 0.2);
    color: #000;
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