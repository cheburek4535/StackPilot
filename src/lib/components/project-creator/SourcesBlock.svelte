<script lang="ts">
  import type { FileEntry } from "$lib/modules/project_creator/types";
  import Icon from "$lib/components/ui/Icon.svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let {
    entries,
  }: {
    entries: FileEntry[];
  } = $props();

  type SourceGroup = {
    source: string;
    certainty: "certain" | "expected" | "unknown";
    count: number;
    files: string[];
  };

  let groups = $derived.by(() => {
    const map = new Map<string, SourceGroup>();
    walkFileEntries(entries, map);
    return Array.from(map.values()).sort((a, b) => {
      const order = { certain: 0, expected: 1, unknown: 2 };
      return order[a.certainty] - order[b.certainty] || b.count - a.count;
    });
  });

  function walkFileEntries(items: FileEntry[], map: Map<string, SourceGroup>) {
    for (const entry of items) {
      // Маркер «… other files» — не файл, а честная пометка неучтённого:
      // в список источников не попадает.
      const isPlaceholder = !entry.is_dir && entry.name.startsWith("…");
      const key = `${entry.source}::${entry.certainty}`;
      if (!map.has(key)) {
        map.set(key, {
          source: entry.source,
          certainty: entry.certainty,
          count: 0,
          files: [],
        });
      }
      const g = map.get(key)!;
      if (!entry.is_dir && !isPlaceholder) {
        g.count++;
        g.files.push(entry.name);
      }
      if (entry.children.length > 0) {
        walkFileEntries(entry.children, map);
      }
    }
  }

  function certaintyLabel(c: string): string {
    switch (c) {
      case "certain": return i18n.t("create.preview.source_our") as TranslationKey;
      case "expected": return i18n.t("create.preview.source_external") as TranslationKey;
      case "unknown": return i18n.t("create.preview.source_runtime") as TranslationKey;
      default: return c;
    }
  }

  function certaintyBadgeClass(c: string): string {
    switch (c) {
      case "certain": return "sb-badge-certain";
      case "expected": return "sb-badge-expected";
      case "unknown": return "sb-badge-unknown";
      default: return "";
    }
  }
</script>

{#if groups.length > 0}
  <div class="sb-container">
    <h4 class="sb-title">{i18n.t("create.preview.sources_title") as TranslationKey}</h4>
    <div class="sb-groups">
      {#each groups as group (group.source + group.certainty)}
        <div class="sb-group">
          <div class="sb-group-header">
            <span class="sb-badge {certaintyBadgeClass(group.certainty)}">
              {certaintyLabel(group.certainty)}
            </span>
            <span class="sb-source-name">{group.source}</span>
            <span class="sb-count">{group.count}</span>
          </div>
          <div class="sb-files">
            {#each group.files.slice(0, 5) as fname}
              <code class="sb-file">{fname}</code>
            {/each}
            {#if group.files.length > 5}
              <span class="sb-more">+{group.files.length - 5}</span>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .sb-container {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .sb-title {
    margin: 0;
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-2);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .sb-groups {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sb-group {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-2);
    border-radius: var(--sp-radius-sm);
  }

  .sb-group-header {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .sb-badge {
    display: inline-flex;
    align-items: center;
    padding: 0.1rem var(--sp-2);
    border-radius: var(--sp-radius-full);
    font-size: 0.6rem;
    font-weight: var(--sp-fw-semibold);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .sb-badge-certain {
    background: rgba(132, 204, 22, 0.14);
    color: var(--sp-success);
  }

  .sb-badge-expected {
    background: rgba(245, 158, 11, 0.14);
    color: var(--sp-warning);
  }

  .sb-badge-unknown {
    background: var(--sp-bg-1);
    color: var(--sp-text-3);
  }

  .sb-source-name {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    flex: 1;
  }

  .sb-count {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .sb-files {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1);
  }

  .sb-file {
    font-family: var(--sp-font-mono);
    font-size: 0.6rem;
    padding: 0.1rem var(--sp-1);
    background: var(--sp-bg-1);
    border-radius: var(--sp-radius-xs);
    color: var(--sp-text-3);
  }

  .sb-more {
    font-size: 0.6rem;
    color: var(--sp-text-3);
    padding: 0.1rem var(--sp-1);
  }
</style>
