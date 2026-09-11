<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  export type TabDef = {
    id: string;
    label: string;
    icon?: IconName;
    disabled?: boolean;
  };

  let {
    tabs,
    value,
    onchange,
  }: {
    tabs: TabDef[];
    value: string;
    onchange?: (id: string) => void;
  } = $props();
</script>

<div class="sp-tabs" role="tablist">
  {#each tabs as tab}
    <button
      type="button"
      role="tab"
      class="sp-tab"
      class:sp-tab-active={tab.id === value}
      aria-selected={tab.id === value}
      disabled={tab.disabled}
      onclick={() => onchange?.(tab.id)}
    >
      {#if tab.icon}
        <Icon name={tab.icon} size={15} />
      {/if}
      <span>{tab.label}</span>
    </button>
  {/each}
</div>

<style>
  .sp-tabs {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: 0.125rem;
    background: var(--sp-surface-grad),
      var(--sp-bg-1);
    border: 1px solid var(--sp-border-faint);
    border-radius: var(--sp-radius-md);
    box-shadow: var(--sp-gloss-top);
    width: max-content;
    max-width: 100%;
    overflow-x: auto;
  }

  .sp-tab {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-3);
    border: none;
    border-radius: var(--sp-radius-sm);
    background: transparent;
    color: var(--sp-text-2);
    font-family: var(--sp-font-sans);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-medium);
    white-space: nowrap;
    cursor: pointer;
    box-shadow: var(--sp-gloss-top);
    transition:
      background-color 0.15s ease,
      color 0.15s ease;
  }

  .sp-tab:hover:not(:disabled) {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }

  .sp-tab-active {
    background: var(--sp-bg-2);
    color: var(--sp-accent);
    box-shadow: var(--sp-gloss-top-strong);
  }

  .sp-tab-active:hover {
    background: var(--sp-bg-2);
    color: var(--sp-accent);
  }

  .sp-tab:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
</style>