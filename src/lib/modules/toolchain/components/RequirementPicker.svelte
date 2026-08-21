<script lang="ts">
  // Селектор требований Build Environment. Источник вариантов —
  // wizard_tree с бэкенда (никаких захардкоженных списков стеков).
  // Выбор — это ВХОД для бэкенд-резолвера, а не авторитет (контракт §1.1).
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import TechIcon from "$lib/components/TechIcon.svelte";
  import type { WizardTreeData } from "$lib/modules/project_creator/types";

  let {
    tree,
    languages = $bindable([]),
    frameworks = $bindable([]),
    tools = $bindable([]),
    localInfra = $bindable([]),
    git = $bindable(false),
    vscode = $bindable(false),
    docker = $bindable(false),
  }: {
    tree: WizardTreeData;
    languages?: string[];
    frameworks?: string[];
    /** id тулов wizard_tree (БД, кэши, тестирование, инфраструктура…). */
    tools?: string[];
    /** Двойные docker-инструменты, выбранные для ЛОКАЛЬНОЙ установки. */
    localInfra?: string[];
    git?: boolean;
    vscode?: boolean;
    docker?: boolean;
  } = $props();

  let frameworkQuery = $state("");

  // ---- Группы тулов по категориям wizard_tree ----

  const TOOL_GROUP_LABELS: Record<string, string> = {
    database: "Базы данных",
    cache: "Кэши и брокеры",
    messaging: "Сообщения и почта",
    observability: "Наблюдаемость",
    container: "Контейнеры",
    testing: "Тестирование",
    baas: "Облако и BaaS",
    orchestration: "Оркестрация",
    etl: "ETL / пайплайны",
    infra: "Инфраструктура",
    tooling: "Сборка и качество кода",
  };

  const toolGroups = $derived.by(() => {
    const groups = new Map<string, typeof tree.tools>();
    for (const t of tree.tools) {
      const key = t.category || "tooling";
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key)!.push(t);
    }
    return [...groups.entries()];
  });

  // ---- Фреймворки: поиск + группировка по стороне ----

  const SIDE_LABELS: Record<string, string> = {
    backend: "Backend",
    frontend: "Frontend",
    either: "Универсальные",
  };

  const filteredFrameworks = $derived.by(() => {
    const q = frameworkQuery.trim().toLowerCase();
    if (!q) return tree.frameworks;
    return tree.frameworks.filter(
      (f) =>
        f.label.toLowerCase().includes(q) ||
        f.id.toLowerCase().includes(q) ||
        f.description.toLowerCase().includes(q),
    );
  });

  const frameworksBySide = $derived.by(() => {
    const groups = new Map<string, typeof filteredFrameworks>();
    for (const f of filteredFrameworks) {
      const side = f.side === "backend" || f.side === "frontend" ? f.side : "either";
      if (!groups.has(side)) groups.set(side, []);
      groups.get(side)!.push(f);
    }
    return [...groups.entries()];
  });

  // ---- Типы проектов: пресеты выбора (маппинги wizard_tree) ----

  function projectTypeActive(typeId: string): boolean {
    const langs = tree.project_language_map[typeId] ?? [];
    const tls = tree.project_tool_map?.[typeId] ?? [];
    return (
      langs.every((l) => languages.includes(l)) &&
      tls.every((t) => tools.includes(t))
    );
  }

  function toggleProjectType(typeId: string): void {
    const langs = tree.project_language_map[typeId] ?? [];
    const tls = tree.project_tool_map?.[typeId] ?? [];
    if (projectTypeActive(typeId)) {
      languages = languages.filter((l) => !langs.includes(l));
      tools = tools.filter((t) => !tls.includes(t));
    } else {
      languages = [...new Set([...languages, ...langs])];
      tools = [...new Set([...tools, ...tls])];
    }
  }

  // ---- Тумблеры ----

  function toggle(list: string[], id: string): string[] {
    return list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
  }

  function isDualTool(toolId: string): boolean {
    return tree.tools.find((t) => t.id === toolId)?.requires_docker === true;
  }

  function selectedCount(...lists: string[][]): number {
    return lists.reduce((n, l) => n + l.length, 0);
  }

  function clearAll(): void {
    languages = [];
    frameworks = [];
    tools = [];
    localInfra = [];
    git = false;
    vscode = false;
    docker = false;
  }
</script>

<div class="picker">
  <!-- ===== Типы проектов ===== -->
  <details class="group" open>
    <summary>
      <span class="group-title">Тип проекта</span>
      <span class="group-hint">добавляет рекомендованные языки и инструменты</span>
    </summary>
    <div class="chips">
      {#each tree.project_types as pt (pt.id)}
        <button
          type="button"
          class="chip chip-type"
          class:active={projectTypeActive(pt.id)}
          onclick={() => toggleProjectType(pt.id)}
          aria-pressed={projectTypeActive(pt.id)}
          title={pt.description}
        >
          {#if projectTypeActive(pt.id)}<Icon name="check" size={12} />{/if}
          {pt.label}
        </button>
      {/each}
    </div>
  </details>

  <!-- ===== Языки ===== -->
  <details class="group" open>
    <summary>
      <span class="group-title">Языки</span>
      {#if languages.length > 0}<Badge tone="violet">{languages.length}</Badge>{/if}
    </summary>
    <div class="chips">
      {#each tree.languages as lang (lang.id)}
        <button
          type="button"
          class="chip"
          class:active={languages.includes(lang.id)}
          onclick={() => (languages = toggle(languages, lang.id))}
          aria-pressed={languages.includes(lang.id)}
        >
          <TechIcon icon={lang.icon} alt="" size="xs" />
          {lang.label}
        </button>
      {/each}
    </div>
  </details>

  <!-- ===== Фреймворки ===== -->
  <details class="group">
    <summary>
      <span class="group-title">Фреймворки</span>
      {#if frameworks.length > 0}<Badge tone="violet">{frameworks.length}</Badge>{/if}
    </summary>
    <div class="fw-search">
      <Icon name="search" size={14} />
      <input
        type="search"
        placeholder="Поиск фреймворка…"
        bind:value={frameworkQuery}
        aria-label="Поиск фреймворка"
      />
    </div>
    {#each frameworksBySide as [side, list] (side)}
      <p class="side-label">{SIDE_LABELS[side] ?? side}</p>
      <div class="chips">
        {#each list as fw (fw.id)}
          {@const compatible =
            fw.languages.length === 0 ||
            fw.languages.some((l) => languages.includes(l))}
          <button
            type="button"
            class="chip"
            class:active={frameworks.includes(fw.id)}
            class:muted={!compatible}
            disabled={!compatible && !frameworks.includes(fw.id)}
            onclick={() => (frameworks = toggle(frameworks, fw.id))}
            aria-pressed={frameworks.includes(fw.id)}
            title={`${fw.description}${compatible ? "" : " · требует язык: " + fw.languages.join(", ")}`}
          >
            <TechIcon icon={fw.icon} alt="" size="xs" />
            {fw.label}
          </button>
        {/each}
      </div>
    {:else}
      <p class="empty-note">Ничего не найдено по запросу «{frameworkQuery}».</p>
    {/each}
  </details>

  <!-- ===== Инфраструктура по категориям ===== -->
  {#each toolGroups as [category, groupTools] (category)}
    <details class="group" open={category === "database"}>
      <summary>
        <span class="group-title">{TOOL_GROUP_LABELS[category] ?? category}</span>
        <Badge tone="neutral">{groupTools.length}</Badge>
      </summary>
      <div class="chips chips-wide">
        {#each groupTools as t (t.id)}
          <div class="tool-chip-wrap">
            <button
              type="button"
              class="chip chip-tool"
              class:active={tools.includes(t.id)}
              onclick={() => (tools = toggle(tools, t.id))}
              aria-pressed={tools.includes(t.id)}
              title={t.description}
            >
              <TechIcon icon={t.icon} alt="" size="xs" />
              {t.label}
              {#if t.requires_docker}
                <span class="docker-mark" title="По умолчанию разворачивается в Docker">D</span>
              {/if}
            </button>
            {#if tools.includes(t.id) && isDualTool(t.id)}
              <label
                class="local-toggle"
                title="Локальная установка вместо docker-compose проекта"
              >
                <input
                  type="checkbox"
                  checked={localInfra.includes(t.id)}
                  onchange={(e) => {
                    const want = e.currentTarget.checked;
                    localInfra = want
                      ? [...new Set([...localInfra, t.id])]
                      : localInfra.filter((x) => x !== t.id);
                  }}
                />
                локально
              </label>
            {/if}
          </div>
        {/each}
      </div>
    </details>
  {/each}

  <!-- ===== Утилиты и флаги ===== -->
  <details class="group" open>
    <summary>
      <span class="group-title">Репозиторий, редактор, Docker</span>
    </summary>
    <div class="switches">
      <label class="switch-row">
        <input type="checkbox" bind:checked={git} />
        <span>Git-репозиторий <span class="hint">(git init)</span></span>
      </label>
      <label class="switch-row">
        <input type="checkbox" bind:checked={vscode} />
        <span>Конфигурация VS Code <span class="hint">(.vscode)</span></span>
      </label>
      <label class="switch-row">
        <input type="checkbox" bind:checked={docker} />
        <span>Docker для окружения проекта</span>
      </label>
    </div>
  </details>

  <div class="picker-footer">
    <Button variant="ghost" size="sm" onclick={clearAll} disabled={selectedCount(languages, frameworks, tools) === 0 && !git && !vscode && !docker}>
      Сбросить выбор
    </Button>
    <span class="picked-count">
      выбрано: {selectedCount(languages, frameworks, tools)}
      {#if git || vscode || docker}
        +{[git, vscode, docker].filter(Boolean).length} опц.
      {/if}
    </span>
  </div>
</div>

<style>
  .picker {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .group {
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    background: var(--sp-bg-1);
    overflow: hidden;
  }

  summary {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-4);
    cursor: pointer;
    user-select: none;
    list-style: none;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  summary::-webkit-details-marker {
    display: none;
  }

  summary::before {
    content: "";
    width: 0;
    height: 0;
    border-left: 5px solid transparent;
    border-right: 5px solid transparent;
    border-top: 6px solid var(--sp-text-3);
    transition: transform 0.15s ease;
  }

  details[open] summary::before {
    transform: rotate(180deg);
  }

  .group-hint {
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-normal);
    color: var(--sp-text-3);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
    padding: 0 var(--sp-4) var(--sp-3);
  }

  .chips-wide {
    gap: var(--sp-2) var(--sp-3);
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-3);
    border-radius: var(--sp-radius-full);
    border: 1px solid var(--sp-border);
    background: var(--sp-bg-2);
    color: var(--sp-text-2);
    font-size: var(--sp-fs-sm);
    cursor: pointer;
    transition:
      background-color 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
    white-space: nowrap;
  }

  .chip:hover:not(:disabled) {
    border-color: var(--sp-border-strong);
    color: var(--sp-text-1);
  }

  .chip.active {
    background: var(--sp-accent-soft);
    border-color: var(--sp-accent-border);
    color: var(--sp-accent);
  }

  .chip.muted {
    opacity: 0.55;
  }

  .chip:disabled {
    cursor: not-allowed;
  }

  .chip-type {
    font-weight: var(--sp-fw-medium);
  }

  .tool-chip-wrap {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
  }

  .docker-mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 0.9rem;
    height: 0.9rem;
    margin-left: var(--sp-1);
    border-radius: var(--sp-radius-xs);
    background: rgba(96, 165, 250, 0.16);
    color: var(--sp-blue);
    font-size: 0.625rem;
    font-weight: var(--sp-fw-bold);
  }

  .local-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    cursor: pointer;
    user-select: none;
  }

  .local-toggle input {
    accent-color: var(--sp-accent-strong);
  }

  .fw-search {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin: 0 var(--sp-4) var(--sp-2);
    padding: var(--sp-1) var(--sp-3);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    background: var(--sp-bg-2);
    color: var(--sp-text-3);
  }

  .fw-search input {
    flex: 1;
    border: none;
    background: transparent;
    color: var(--sp-text-1);
    font-size: var(--sp-fs-sm);
    outline: none;
  }

  .side-label {
    margin: var(--sp-1) var(--sp-4) 0;
    padding: 0;
    font-size: var(--sp-fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--sp-text-3);
  }

  .empty-note {
    margin: 0 var(--sp-4) var(--sp-3);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
    font-style: italic;
  }

  .switches {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    padding: 0 var(--sp-4) var(--sp-3);
  }

  .switch-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-1);
    cursor: pointer;
    user-select: none;
  }

  .switch-row input {
    accent-color: var(--sp-accent-strong);
  }

  .hint {
    color: var(--sp-text-3);
    font-size: var(--sp-fs-xs);
  }

  .picker-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding-top: var(--sp-1);
  }

  .picked-count {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }
</style>
