<script lang="ts">
  import Icon from "$lib/components/ui/Icon.svelte";
  import type { IconName } from "$lib/components/ui/icons";
  import type { Snippet } from "svelte";

  let {
    id = "",
    title,
    description = "",
    icon,
    count = 0,
    badgeText = "",
    badgeTone = "neutral",
    children,
  }: {
    id?: string;
    title: string;
    description?: string;
    icon: IconName;
    count?: number;
    badgeText?: string;
    badgeTone?: "neutral" | "lime" | "amber" | "cyan" | "violet";
    children?: Snippet;
  } = $props();
</script>

<section class="tc-section" {id}>
  <header class="tc-section-header">
    <div class="tc-section-lead">
      <div class="tc-section-icon-box" aria-hidden="true">
        <Icon name={icon} size={16} />
      </div>
      <div class="tc-section-info">
        <div class="tc-section-title-row">
          <h2 class="tc-section-title">{title}</h2>
          {#if badgeText}
            <span class="tc-section-badge tc-badge-{badgeTone}">{badgeText}</span>
          {:else if count > 0}
            <span class="tc-section-count">{count}</span>
          {/if}
        </div>
        {#if description}
          <p class="tc-section-desc">{description}</p>
        {/if}
      </div>
    </div>
  </header>

  <div class="tc-section-body">
    {@render children?.()}
  </div>
</section>

<style>
  .tc-section {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    margin-bottom: var(--sp-6);
  }

  .tc-section:last-child {
    margin-bottom: 0;
  }

  .tc-section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-bottom: var(--sp-2);
    border-bottom: 1px solid var(--sp-border-subtle, rgba(255, 255, 255, 0.07));
  }

  .tc-section-lead {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
    min-width: 0;
  }

  .tc-section-icon-box {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    border-radius: var(--sp-radius-md);
    background: var(--sp-surface-2, rgba(255, 255, 255, 0.04));
    border: 1px solid var(--sp-border, rgba(255, 255, 255, 0.1));
    color: var(--sp-accent, #6366f1);
    flex-shrink: 0;
    margin-top: 1px;
  }

  .tc-section-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .tc-section-title-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }

  .tc-section-title {
    margin: 0;
    font-size: var(--sp-fs-md, 0.95rem);
    font-weight: 600;
    color: var(--sp-text-1);
    letter-spacing: -0.01em;
  }

  .tc-section-desc {
    margin: 0;
    font-size: var(--sp-fs-xs, 0.75rem);
    color: var(--sp-text-3);
    line-height: 1.35;
  }

  .tc-section-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 0.7rem;
    font-weight: 600;
    color: var(--sp-text-2);
    background: var(--sp-surface-2, rgba(255, 255, 255, 0.06));
    border: 1px solid var(--sp-border, rgba(255, 255, 255, 0.08));
    padding: 1px 7px;
    border-radius: 999px;
  }

  .tc-section-badge {
    display: inline-flex;
    align-items: center;
    font-size: 0.7rem;
    font-weight: 600;
    padding: 1px 8px;
    border-radius: 999px;
  }

  .tc-badge-amber {
    background: rgba(245, 158, 11, 0.15);
    color: #f59e0b;
    border: 1px solid rgba(245, 158, 11, 0.3);
  }

  .tc-badge-lime {
    background: rgba(132, 204, 22, 0.15);
    color: #84cc16;
    border: 1px solid rgba(132, 204, 22, 0.3);
  }

  .tc-badge-cyan {
    background: rgba(6, 182, 212, 0.15);
    color: #06b6d4;
    border: 1px solid rgba(6, 182, 212, 0.3);
  }

  .tc-badge-violet {
    background: rgba(139, 92, 246, 0.15);
    color: #8b5cf6;
    border: 1px solid rgba(139, 92, 246, 0.3);
  }

  .tc-badge-neutral {
    background: var(--sp-surface-2);
    color: var(--sp-text-2);
    border: 1px solid var(--sp-border);
  }

  .tc-section-body {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
</style>
