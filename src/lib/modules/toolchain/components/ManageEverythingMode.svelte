<script lang="ts">
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
  import type { CatalogSort, HealthState } from "../types";

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

  const HEALTH_OPTIONS: { kind: HealthState["kind"]; label: string }[] = [
    { kind: "healthy", label: "Здоров" },
    { kind: "degraded", label: "Деградация" },
    { kind: "unhealthy", label: "Нездоров" },
    { kind: "not_checked", label: "Не проверялся" },
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

  const SORT_OPTIONS: { value: CatalogSort; label: string }[] = [
    { value: "status", label: "Проблемные сверху" },
    { value: "name_asc", label: "Имя А→Я" },
    { value: "name_desc", label: "Имя Я→А" },
    { value: "category", label: "По категории" },
    { value: "catalog", label: "Порядок каталога" },
  ];
</script>

<div class="manage">
  <!-- ===== Панель поиска и сортировки ===== -->
  <div class="toolbar">
    <div class="search">
      <Icon name="search" size={16} />
      <input
        type="search"
        placeholder="Поиск по имени, id или категории…"
        value={toolchain.filters.search}
        oninput={(e) => toolchain.setFilters({ search: e.currentTarget.value })}
        aria-label="Поиск инструментов"
      />
      {#if toolchain.filters.search}
        <IconButton
          icon="x"
          label="Очистить поиск"
          size="sm"
          onclick={() => toolchain.setFilters({ search: "" })}
        />
      {/if}
    </div>

    <label class="sort">
      <span class="sort-label">Сортировка</span>
      <select
        value={sort}
        onchange={(e) => (sort = e.currentTarget.value as CatalogSort)}
        aria-label="Порядок сортировки"
      >
        {#each SORT_OPTIONS as opt (opt.value)}
          <option value={opt.value}>{opt.label}</option>
        {/each}
      </select>
    </label>

    <div class="rail-toggle-wrap">
      <Button variant="ghost" size="sm" icon={railOpen ? "x" : "layers"} onclick={() => (railOpen = !railOpen)}>
        Фильтры
      </Button>
    </div>
  </div>

  <div class="content">
    <!-- ===== Рейло фильтров ===== -->
    <aside class="rail" class:rail-open={railOpen} aria-label="Фильтры каталога">
      <div class="rail-head">
        <span>Фильтры</span>
        <Button variant="ghost" size="sm" onclick={() => toolchain.resetFilters()}>
          Сбросить всё
        </Button>
      </div>

      <div class="rail-body">
        <!-- Состояния -->
        <fieldset>
          <legend>Состояние</legend>
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
          <legend>Здоровье</legend>
          {#each HEALTH_OPTIONS as opt (opt.kind)}
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
            </label>
          {/each}
        </fieldset>

        <!-- Категории -->
        <fieldset>
          <legend>Категории</legend>
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
          <legend>Происхождение</legend>
          {#each allProvenanceKinds() as { kind, info } (kind)}
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
            </label>
          {/each}
        </fieldset>

        <!-- Возможности платформы -->
        <fieldset>
          <legend>Возможности</legend>
          {#each capabilityOptions as flag (flag)}
            <label class="filter-row">
              <input
                type="checkbox"
                checked={toolchain.filters.capabilities.includes(flag)}
                onchange={() =>
                  toolchain.setFilters({
                    capabilities: toggleInArray(toolchain.filters.capabilities, flag),
                  })}
              />
              <span class="filter-label">{capabilityLabel(flag)}</span>
            </label>
          {/each}
        </fieldset>

        <!-- Режим исполнения -->
        <fieldset>
          <legend>Исполнение</legend>
          <label class="filter-row">
            <input
              type="checkbox"
              checked={toolchain.filters.execution_modes.includes("host")}
              onchange={() =>
                toolchain.setFilters({
                  execution_modes: toggleInArray(toolchain.filters.execution_modes, "host"),
                })}
            />
            <span class="filter-label">На хосте</span>
          </label>
          <label class="filter-row">
            <input
              type="checkbox"
              checked={toolchain.filters.execution_modes.includes("docker")}
              onchange={() =>
                toolchain.setFilters({
                  execution_modes: toggleInArray(toolchain.filters.execution_modes, "docker"),
                })}
            />
            <span class="filter-label">В Docker</span>
          </label>
          <label class="filter-row">
            <input
              type="checkbox"
              checked={toolchain.filters.manual_only}
              onchange={() => toolchain.setFilters({ manual_only: !toolchain.filters.manual_only })}
            />
            <span class="filter-label">Только ручная установка</span>
          </label>
        </fieldset>

        <!-- Быстрые фильтры -->
        <fieldset>
          <legend>Быстрые фильтры</legend>
          <label class="filter-row">
            <input
              type="checkbox"
              checked={toolchain.filters.update_only}
              onchange={() => toolchain.setFilters({ update_only: !toolchain.filters.update_only })}
            />
            <span class="filter-label">Только обновления</span>
          </label>
          <label class="filter-row">
            <input
              type="checkbox"
              checked={toolchain.filters.admin_only}
              onchange={() => toolchain.setFilters({ admin_only: !toolchain.filters.admin_only })}
            />
            <span class="filter-label">Нужны права администратора</span>
          </label>
        </fieldset>
      </div>
    </aside>

    <!-- ===== Сетка карточек ===== -->
    <div class="results">
      <p class="results-count" role="status">
        {visibleTools.length} из {totalTools} инструментов
        {#if visibleTools.length !== totalTools}
          <button type="button" class="link-btn" onclick={() => toolchain.resetFilters()}>
            сбросить фильтры
          </button>
        {/if}
      </p>

      {#if toolchain.snapshotLoading && !snapshot}
        <LoadingState label="Загружаем состояние окружения…" />
      {:else if toolchain.snapshotError && !snapshot}
        <ErrorState
          title="Не удалось загрузить состояние окружения"
          message={toolchain.snapshotError}
          retry={() => void toolchain.refreshSnapshot()}
        />
      {:else if !snapshot || snapshot.tools.length === 0}
        <EmptyState
          icon="wrench"
          title="Данных о машине пока нет"
          description="Запустите первый диагностический скан — он ничего не устанавливает и занимает меньше двух минут."
        >
          {#snippet action()}
            <Button variant="primary" icon="refresh" loading={toolchain.snapshotLoading} onclick={() => void toolchain.ensureScanRunning()}>
              Запустить скан
            </Button>
          {/snippet}
        </EmptyState>
      {:else if visibleTools.length === 0}
        <EmptyState
          compact
          icon="search"
          title="Ничего не найдено"
          description="По текущим фильтрам и поиску инструменты не найдены. Попробуйте ослабить условия."
        >
          {#snippet action()}
            <Button variant="secondary" size="sm" icon="refresh" onclick={() => toolchain.resetFilters()}>
              Сбросить фильтры
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
            />
          {/each}
        </div>
      {/if}

      {#if snapshot && snapshot.tools.some((t) => t.state.kind === "scan_pending")}
        <p class="scan-note" role="status">
          <Icon name="clock" size={13} />
          Часть инструментов ещё проверяется — карточки дополнятся по мере сканирования.
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
