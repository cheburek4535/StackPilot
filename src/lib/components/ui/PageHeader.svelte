<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  let {
    title,
    description,
    icon,
    actions,
    children,
  }: {
    title: string;
    description?: string;
    icon?: IconName;
    actions?: Snippet;
    children?: Snippet;
  } = $props();
</script>

<header class="sp-page-header">
  <div class="sp-page-header-main">
    {#if icon}
      <span class="sp-page-header-icon" aria-hidden="true">
        <Icon name={icon} size={20} />
      </span>
    {/if}
    <div class="sp-page-header-text">
      <h1 class="sp-page-header-title">{title}</h1>
      {#if description}
        <p class="sp-page-header-desc">{description}</p>
      {/if}
    </div>
  </div>
  {#if actions}
    <div class="sp-page-header-actions">{@render actions()}</div>
  {/if}
</header>

{#if children}
  <div class="sp-page-header-content">{@render children()}</div>
{/if}

<style>
  .sp-page-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--sp-4);
    margin-bottom: var(--sp-6);
  }

  .sp-page-header-main {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
  }

  .sp-page-header-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2.5rem;
    height: 2.5rem;
    flex: 0 0 auto;
    border-radius: var(--sp-radius-lg);
    color: var(--sp-text-2);
    background: var(--sp-surface-grad),
      var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    box-shadow: var(--sp-gloss-top), var(--sp-shadow-1);
  }

  .sp-page-header-text {
    min-width: 0;
  }

  .sp-page-header-title {
    margin: 0;
    font-size: var(--sp-fs-2xl);
    font-weight: var(--sp-fw-bold);
    letter-spacing: -0.03em;
    line-height: var(--sp-lh-tight);
    color: var(--sp-text-1);
    text-shadow: 0 0 28px var(--sp-accent-glow);
  }

  .sp-page-header-desc {
    margin: var(--sp-1) 0 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
    line-height: var(--sp-lh-normal);
  }

  .sp-page-header-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex: 0 0 auto;
  }

  .sp-page-header-content {
    margin-bottom: var(--sp-6);
  }
</style>