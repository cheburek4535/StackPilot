<script lang="ts">
  import type { Snippet } from "svelte";

  export type CardVariant = "glass" | "elevated" | "outline";
  export type CardPadding = "sm" | "md" | "lg" | "none";

  let {
    variant = "elevated",
    padding = "md",
    class: klass = "",
    title,
    description,
    actions,
    children,
  }: {
    variant?: CardVariant;
    padding?: CardPadding;
    class?: string;
    title?: string;
    description?: string;
    actions?: Snippet;
    children: Snippet;
  } = $props();
</script>

<section class="sp-card sp-card-{variant} sp-card-pad-{padding} {klass}">
  {#if title || actions}
    <header class="sp-card-header">
      <div class="sp-card-heading">
        {#if title}
          <h3 class="sp-card-title">{title}</h3>
        {/if}
        {#if description}
          <p class="sp-card-desc">{description}</p>
        {/if}
      </div>
      {#if actions}
        <div class="sp-card-actions">{@render actions()}</div>
      {/if}
    </header>
  {/if}
  <div class="sp-card-body">{@render children()}</div>
</section>

<style>
  .sp-card {
    border-radius: var(--sp-radius-lg);
    border: 1px solid var(--sp-border);
    min-width: 0;
    transition: border-color 0.2s ease, box-shadow 0.2s ease;
  }

  .sp-card-glass {
    background: var(--sp-glass-bg);
    box-shadow:
      var(--sp-gloss-top),
      var(--sp-shadow-1);
  }

  .sp-card-elevated {
    background: var(--sp-surface-grad),
      var(--sp-bg-1);
    box-shadow:
      var(--sp-gloss-top),
      var(--sp-shadow-2);
  }

  .sp-card-elevated:hover {
    border-color: var(--sp-border-strong);
  }

  .sp-card-outline {
    background: transparent;
    box-shadow: var(--sp-gloss-top);
  }

  .sp-card-pad-sm .sp-card-body {
    padding: var(--sp-3);
  }

  .sp-card-pad-md .sp-card-body {
    padding: var(--sp-5);
  }

  .sp-card-pad-lg .sp-card-body {
    padding: var(--sp-6);
  }

  .sp-card-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-4) var(--sp-5) 0;
  }

  .sp-card-pad-none .sp-card-header {
    padding: var(--sp-4) var(--sp-4) 0;
  }

  .sp-card-heading {
    min-width: 0;
  }

  .sp-card-title {
    margin: 0;
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    line-height: var(--sp-lh-tight);
  }

  .sp-card-desc {
    margin: var(--sp-1) 0 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
    line-height: var(--sp-lh-normal);
  }

  .sp-card-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex: 0 0 auto;
  }

  .sp-card-pad-sm .sp-card-header {
    padding: var(--sp-3) var(--sp-3) 0;
  }
</style>