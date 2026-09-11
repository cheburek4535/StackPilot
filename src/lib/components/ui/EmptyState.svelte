<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  let {
    icon = "folder",
    title,
    description,
    action,
    compact = false,
  }: {
    icon?: IconName;
    title: string;
    description?: string;
    action?: Snippet;
    compact?: boolean;
  } = $props();
</script>

<div class="sp-empty" class:sp-empty-compact={compact}>
  <span class="sp-empty-icon" aria-hidden="true">
    <Icon name={icon} size={compact ? 18 : 24} />
  </span>
  <p class="sp-empty-title">{title}</p>
  {#if description}
    <p class="sp-empty-desc">{description}</p>
  {/if}
  {#if action}
    <div class="sp-empty-action">{@render action()}</div>
  {/if}
</div>

<style>
  .sp-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    gap: var(--sp-2);
    padding: var(--sp-10) var(--sp-6);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    background: var(--sp-surface-grad),
      var(--sp-bg-1);
    box-shadow: var(--sp-gloss-top), var(--sp-shadow-1);
  }

  .sp-empty-compact {
    padding: var(--sp-6) var(--sp-4);
  }

  .sp-empty-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 3.25rem;
    height: 3.25rem;
    border-radius: var(--sp-radius-full);
    background: var(--sp-surface-grad),
      var(--sp-bg-2);
    color: var(--sp-text-2);
    border: 1px solid var(--sp-border);
    box-shadow: var(--sp-gloss-top-strong);
    margin-bottom: var(--sp-1);
  }

  .sp-empty-compact .sp-empty-icon {
    width: 2.5rem;
    height: 2.5rem;
  }

  .sp-empty-title {
    margin: 0;
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-empty-desc {
    margin: 0;
    max-width: 30rem;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
    line-height: var(--sp-lh-normal);
  }

  .sp-empty-action {
    margin-top: var(--sp-2);
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
</style>