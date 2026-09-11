<script lang="ts">
  import type { FileEntry, FileCertainty } from "$lib/modules/project_creator/types";
  import Icon from "$lib/components/ui/Icon.svelte";
  import type { IconName } from "$lib/components/ui/icons";
  import FileTree from "./FileTree.svelte";

  let {
    entries = [],
    level = 0,
    onselect,
  }: {
    entries?: FileEntry[];
    level?: number;
    onselect?: (entry: FileEntry) => void;
  } = $props();

  let safeEntries = $derived(Array.isArray(entries) ? entries : []);

  let expanded = $state<Record<string, boolean>>({});

  // Корневые папки раскрыты по умолчанию (как в VS Code), вложенные — свёрнуты.
  // Сброс при каждом новом дереве (смена стека/фильтров) — свежий старт.
  $effect(() => {
    const rootDirs = safeEntries.filter((e) => e.is_dir).map((e) => e.path);
    if (rootDirs.length === 0) return;
    expanded = Object.fromEntries(rootDirs.map((p) => [p, true]));
  });

  function toggle(name: string) {
    expanded[name] = !expanded[name];
  }

  function handleClick(entry: FileEntry) {
    if (entry.is_dir) {
      toggle(entry.path);
    } else {
      onselect?.(entry);
    }
  }

  function certaintyColor(c: FileCertainty): string {
    switch (c) {
      case "certain": return "ft-certainty-certain";
      case "expected": return "ft-certainty-expected";
      case "unknown": return "ft-certainty-unknown";
    }
  }

  function certaintyIcon(c: FileCertainty): IconName {
    switch (c) {
      case "certain": return "check";
      case "expected": return "alert";
      case "unknown": return "info";
    }
  }

  function iconName(entry: FileEntry): IconName {
    return entry.is_dir ? "folder" : "file";
  }
</script>

<ul class="ft-list" class:ft-root={level === 0}>
  {#each safeEntries as entry (entry.path)}
    <li class="ft-item" style:padding-left="{level * 14}px">
      <button
        class="ft-row {certaintyColor(entry.certainty)}"
        onclick={() => handleClick(entry)}
        title={entry.warning ?? entry.source}
        aria-expanded={entry.is_dir ? expanded[entry.path] : undefined}
      >
        {#if entry.is_dir}
          <span class="ft-chevron" class:ft-chevron-open={expanded[entry.path]}>
            <Icon name="chevronRight" size={12} />
          </span>
        {:else}
          <span class="ft-chevron ft-chevron-placeholder"></span>
        {/if}
        <Icon name={iconName(entry)} size={14} class="ft-file-icon" />
        <span class="ft-name">{entry.name}</span>
        {#if entry.is_dir}
          <span class="ft-count">{(entry.children ?? []).length}</span>
        {/if}
        <span class="ft-badge {certaintyColor(entry.certainty)}" title={entry.source}>
          <Icon name={certaintyIcon(entry.certainty)} size={10} />
        </span>
      </button>
      {#if entry.is_dir && expanded[entry.path]}
        <FileTree entries={entry.children ?? []} level={level + 1} {onselect} />
      {/if}
    </li>
  {/each}
</ul>

<style>
  .ft-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .ft-root {
    padding: var(--sp-2) 0;
  }

  .ft-item {
    position: relative;
  }

  .ft-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    width: 100%;
    padding: 4px var(--sp-2);
    border: none;
    background: none;
    color: var(--sp-text-1);
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-sm);
    text-align: left;
    cursor: pointer;
    border-radius: var(--sp-radius-sm);
    transition: background 0.1s;
  }

  .ft-row:hover {
    background: var(--sp-bg-2);
  }

  .ft-row:focus-visible {
    outline: 2px solid var(--sp-accent);
    outline-offset: -2px;
  }

  .ft-chevron {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 12px;
    transition: transform 0.15s;
    color: var(--sp-text-3);
    flex-shrink: 0;
  }

  .ft-chevron-open {
    transform: rotate(90deg);
  }

  .ft-chevron-placeholder {
    visibility: hidden;
  }

  .ft-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ft-count {
    color: var(--sp-text-3);
    font-size: 0.65rem;
    min-width: 16px;
    text-align: right;
  }

  .ft-badge {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border-radius: var(--sp-radius-full);
    flex-shrink: 0;
  }

  .ft-certainty-certain {
    color: var(--sp-success);
  }

  .ft-certainty-expected {
    color: var(--sp-amber);
  }

  .ft-certainty-unknown {
    color: var(--sp-text-3);
  }

  .ft-badge.ft-certainty-certain {
    background: var(--sp-success-soft);
  }

  .ft-badge.ft-certainty-expected {
    background: var(--sp-warning-border);
  }

  .ft-badge.ft-certainty-unknown {
    background: var(--sp-bg-2);
  }

  :global(.ft-file-icon) {
    flex-shrink: 0;
    color: var(--sp-text-3);
  }

  .ft-row.ft-certainty-certain :global(.ft-file-icon) {
    color: var(--sp-success);
  }

  .ft-row.ft-certainty-expected :global(.ft-file-icon) {
    color: var(--sp-amber);
  }
</style>
